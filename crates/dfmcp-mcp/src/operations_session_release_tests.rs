//! Public lifecycle handlers over the same injected native source as query tests.
//! Retained Arcs deliberately outlive close; cleanup must release their resources.
use super::*;
use dfmcp_core::GameTick;

struct CountedSource {
    calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}
impl OperationsSource for CountedSource {
    fn read(&mut self, _: Duration) -> Result<LiveOperationsObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(error(
            ErrorCode::AdapterUnavailable,
            "unexpected close read",
        ))
    }
    fn poisoned(&self) -> bool {
        false
    }
    fn fence(&mut self) {}
}
impl Drop for CountedSource {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn request_close(session: &Registered, discard: Option<bool>) -> Result<Value> {
    decode(&fortress_cancel(
        session.handle(),
        Some("session".into()),
        discard,
    ))
}

fn retain_query_work(session: &Registered) -> Result<()> {
    let captured = ask(
        session,
        json!({"kind":"capture","key":"close-baseline","max_game_ticks":100,
        "select":{"kind":"entities","kinds":["item"],"fields":["stack_size"]}}),
    )?;
    assert_eq!(captured["ok"], true, "{captured}");
    let watch = ask(
        session,
        json!({"kind":"watch","key":"close-watch",
        "condition":{"op":"tick_at_least","value":105u64*403200+100},
        "deadline_tick":105u64*403200+200,"stable_observations":1}),
    )?;
    assert_eq!(watch["ok"], true, "{watch}");
    Ok(())
}

#[test]
fn close_releases_source_and_capacity_even_while_old_arcs_remain() -> Result<()> {
    let _serial = crate::test_serial();
    let session = full()?;
    let stale = resolve(session.handle())?;
    let context = lock(&stale)?.context()?;
    let drops = Arc::new(AtomicUsize::new(0));
    lock(&stale)?.source = Box::new(CountedSource {
        calls: Arc::clone(&session.calls),
        drops: Arc::clone(&drops),
    });
    let encoded = fortress_cancel(session.handle(), Some("session".into()), None);
    let result = decode(&encoded)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["release_only"], true);
    assert_eq!(result["native_captures"], 0);
    assert_eq!(result["game_effects_cancelled"], false);
    assert!(result["agent_turn"]["anchor"].is_null());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    {
        let mut old = lock(&stale)?;
        let request = old.request;
        assert!(matches!(old.context(), Err(e) if e.code == ErrorCode::SessionNotFound));
        assert_eq!(old.request, request);
        assert!(matches!(old.refresh(&context), Err(e) if e.code == ErrorCode::SessionNotFound));
        assert!(old.state.snapshot().is_none());
        assert!(old.journal.is_none());
        assert!(old.grants.is_empty());
        assert!(!old._slot.held);
    }
    assert_eq!(
        fortress_cancel(session.handle(), Some("session".into()), Some(true)),
        encoded
    );
    assert_eq!(
        decode(&fortress_observe(session.handle()))?["error"]["code"],
        "session_not_found"
    );
    let mut handles = vec![stale];
    for _ in 0..MAX_SESSIONS + 1 {
        let next = full()?;
        handles.push(resolve(next.handle())?);
        assert_eq!(request_close(&next, None)?["ok"], true);
    }
    drop(handles);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn consent_and_full_receipt_preflight_preserve_work_until_success() -> Result<()> {
    let _serial = crate::test_serial();
    let session = full()?;
    let other = full()?;
    retain_query_work(&session)?;
    retain_query_work(&other)?;
    let watches = ask(&session, json!({"kind":"watches"}))?["records"].clone();
    let baselines = ask(&session, json!({"kind":"baselines"}))?["baselines"].clone();
    let other_watches = ask(&other, json!({"kind":"watches"}))?["records"].clone();
    let refused = request_close(&session, None)?;
    assert_eq!(refused["error"]["code"], "conflict");
    assert!(refused["agent_turn"]["anchor"].is_null());
    let handle = resolve(session.handle())?;
    lock(&handle)?.budget.max_output_tokens = 1;
    let too_small = request_close(&session, Some(true))?;
    lock(&handle)?.budget.max_output_tokens = 8192;
    assert_eq!(too_small["error"]["code"], "budget_exceeded");
    assert_eq!(
        ask(&session, json!({"kind":"watches"}))?["records"],
        watches
    );
    assert_eq!(
        ask(&session, json!({"kind":"baselines"}))?["baselines"],
        baselines
    );
    assert!(lock(&handle)?._slot.held);
    assert!(!lock(&handle)?.source.closed());
    let result = request_close(&session, Some(true))?;
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
        ask(&other, json!({"kind":"watches"}))?["records"],
        other_watches
    );
    assert_eq!(request_close(&other, None)?["error"]["code"], "conflict");
    assert_eq!(request_close(&other, Some(true))?["ok"], true);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(other.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn fenced_or_unauthorized_sessions_can_release_without_observing() -> Result<()> {
    let _serial = crate::test_serial();
    for case in 0..4 {
        let session = full()?;
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
        let result = request_close(&session, None)?;
        assert_eq!(result["ok"], true, "case {case}: {result}");
        assert_eq!(result["game_state_observed"], false);
        assert!(result.get("anchor").is_none());
        assert!(result["agent_turn"]["anchor"].is_null());
        assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[test]
#[allow(clippy::panic)]
fn poisoned_session_can_close_without_reusing_its_state() -> Result<()> {
    let _serial = crate::test_serial();
    let session = full()?;
    let handle = resolve(session.handle())?;
    let poison = Arc::clone(&handle);
    let panicked = std::thread::spawn(move || {
        if let Ok(_guard) = poison.lock() {
            panic!("injected operations foreground panic");
        }
    })
    .join()
    .is_err();
    assert!(panicked);
    assert_eq!(decode(&fortress_observe(session.handle()))?["ok"], false);
    let result = request_close(&session, None)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["session_mutex_poisoned"], true);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert!(handle.is_poisoned());
    Ok(())
}

#[test]
fn invalid_scope_and_foreign_handles_cannot_release_a_session() -> Result<()> {
    let _serial = crate::test_serial();
    let session = full()?;
    assert_eq!(
        decode(&fortress_cancel(session.handle(), None, None))?["error"]["code"],
        "capability_denied"
    );
    for (scope, discard) in [(None, Some(true)), (Some("game".into()), None)] {
        assert_eq!(
            decode(&fortress_cancel(session.handle(), scope, discard))?["error"]["code"],
            "invalid_request"
        );
    }
    for handle in [
        None,
        Some("bad-handle".into()),
        Some(format!("{:032x}", (1u128 << 127) | (1u128 << 60) | 1)),
    ] {
        assert_eq!(
            decode(&fortress_cancel(handle, Some("session".into()), None))?["error"]["code"],
            "invalid_request"
        );
    }
    assert!(resolve(session.handle()).is_ok());
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(request_close(&session, None)?["ok"], true);
    let definition = FortressCancel.definition();
    assert_eq!(definition.name, "fortress_cancel");
    for field in ["session_id", "scope", "discard_process_local_work"] {
        assert!(definition.input_schema["properties"].get(field).is_some());
        assert!(
            !definition.input_schema["required"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|value| value == field))
        );
    }
    Ok(())
}
