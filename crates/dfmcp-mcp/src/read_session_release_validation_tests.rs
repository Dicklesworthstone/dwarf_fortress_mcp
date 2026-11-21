//! Independent lifecycle regressions against the exact production custody helper.
//! These use real mutexes, registries, and sockets; no native DFHack endpoint or
//! replacement runtime implementation is involved.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, mpsc};
use std::time::{Duration, Instant};

struct OwnedResource {
    value: usize,
    drops: Arc<AtomicUsize>,
}

impl Drop for OwnedResource {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn failure(message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn held<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>> {
    mutex.lock().map_err(|_| failure("fixture mutex poisoned"))
}

fn registered(
    id: SessionId,
    drops: &Arc<AtomicUsize>,
) -> (
    Arc<Mutex<SessionRegistry<OwnedResource>>>,
    SessionHandle<OwnedResource>,
) {
    let handle = Arc::new(Mutex::new(Some(OwnedResource {
        value: 1,
        drops: Arc::clone(drops),
    })));
    let registry = Arc::new(Mutex::new(BTreeMap::from([(id, Arc::clone(&handle))])));
    (registry, handle)
}

fn await_resolved<T>(handle: &SessionHandle<T>) -> Result<()> {
    // One Arc belongs to this test and one to the registry. The third proves
    // close has resolved this exact cell and is waiting for its foreground owner.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Arc::strong_count(handle) < 3 {
        if Instant::now() >= deadline {
            return Err(failure("close did not resolve the held session"));
        }
        std::thread::yield_now();
    }
    Ok(())
}

#[test]
fn simultaneous_close_publishes_one_receipt_and_drops_resources_once() -> Result<()> {
    let id = SessionId::new(401);
    let drops = Arc::new(AtomicUsize::new(0));
    let prepared = Arc::new(AtomicUsize::new(0));
    let (registry, stale) = registered(id, &drops);
    let receipts = Arc::new(Mutex::new(CloseReceipts::new()));
    let start = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let registry = Arc::clone(&registry);
        let receipts = Arc::clone(&receipts);
        let prepared = Arc::clone(&prepared);
        let start = Arc::clone(&start);
        workers.push(std::thread::spawn(move || {
            start.wait();
            close(id, &registry, &receipts, |_, poisoned| {
                assert!(!poisoned);
                let sequence = prepared.fetch_add(1, Ordering::SeqCst) + 1;
                Ok(format!("complete-receipt-{sequence}"))
            })
        }));
    }
    start.wait();
    for worker in workers {
        assert_eq!(
            worker
                .join()
                .map_err(|_| failure("close worker panicked"))??,
            "complete-receipt-1"
        );
    }
    assert_eq!(prepared.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(held(&registry)?.is_empty());
    assert_eq!(held(&receipts)?.len(), 1);
    assert!(held(&stale)?.is_none());
    assert!(matches!(
        require_open(&mut *held(&stale)?),
        Err(error) if error.code == ErrorCode::SessionNotFound
    ));
    drop(stale);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn foreground_owner_completes_before_close_prepares_its_receipt() -> Result<()> {
    let id = SessionId::new(402);
    let drops = Arc::new(AtomicUsize::new(0));
    let (registry, stale) = registered(id, &drops);
    let receipts = Arc::new(Mutex::new(CloseReceipts::new()));
    let mut foreground = held(&stale)?;
    let worker_registry = Arc::clone(&registry);
    let worker_receipts = Arc::clone(&receipts);
    let worker = std::thread::spawn(move || {
        close(id, &worker_registry, &worker_receipts, |session, _| {
            assert_eq!(session.value, 77);
            Ok("after-foreground".to_owned())
        })
    });
    let resolved = await_resolved(&stale);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert!(held(&receipts)?.is_empty());
    require_open(&mut foreground)?.value = 77;
    drop(foreground);
    let result = worker
        .join()
        .map_err(|_| failure("close worker panicked"))?;
    resolved?;
    assert_eq!(result?, "after-foreground");
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn replaced_registry_owner_cannot_be_closed_through_a_previously_resolved_arc() -> Result<()> {
    let id = SessionId::new(403);
    let original_drops = Arc::new(AtomicUsize::new(0));
    let replacement_drops = Arc::new(AtomicUsize::new(0));
    let (registry, original) = registered(id, &original_drops);
    let receipts = Arc::new(Mutex::new(CloseReceipts::new()));
    let prepared = Arc::new(AtomicUsize::new(0));
    let foreground = held(&original)?;
    let worker_registry = Arc::clone(&registry);
    let worker_receipts = Arc::clone(&receipts);
    let worker_prepared = Arc::clone(&prepared);
    let worker = std::thread::spawn(move || {
        close(id, &worker_registry, &worker_receipts, |_, _| {
            worker_prepared.fetch_add(1, Ordering::SeqCst);
            Ok("must-not-publish".to_owned())
        })
    });
    let resolved = await_resolved(&original);
    let replacement = Arc::new(Mutex::new(Some(OwnedResource {
        value: 2,
        drops: Arc::clone(&replacement_drops),
    })));
    held(&registry)?.insert(id, Arc::clone(&replacement));
    drop(foreground);
    let result = worker
        .join()
        .map_err(|_| failure("close worker panicked"))?;
    resolved?;
    assert!(matches!(result, Err(error) if error.code == ErrorCode::SessionNotFound));
    assert_eq!(prepared.load(Ordering::SeqCst), 0);
    assert_eq!(original_drops.load(Ordering::SeqCst), 0);
    assert_eq!(replacement_drops.load(Ordering::SeqCst), 0);
    assert!(held(&receipts)?.is_empty());
    assert!(
        held(&registry)?
            .get(&id)
            .is_some_and(|value| Arc::ptr_eq(value, &replacement))
    );
    assert_eq!(require_open(&mut *held(&original)?)?.value, 1);
    assert_eq!(require_open(&mut *held(&replacement)?)?.value, 2);
    Ok(())
}

#[test]
fn every_preparation_refusal_preserves_exact_session_custody() -> Result<()> {
    let id = SessionId::new(404);
    let drops = Arc::new(AtomicUsize::new(0));
    let (registry, stale) = registered(id, &drops);
    let receipts = Mutex::new(CloseReceipts::new());
    for code in [
        ErrorCode::Conflict,
        ErrorCode::BudgetExceeded,
        ErrorCode::InternalInvariantViolation,
    ] {
        let result = close(id, &registry, &receipts, |session, poisoned| {
            assert!(!poisoned);
            assert_eq!(session.value, 1);
            Err(dfmcp_core::DfmcpError::new(
                code,
                "prepared response refused",
            ))
        });
        assert!(matches!(result, Err(error) if error.code == code));
        assert!(
            held(&registry)?
                .get(&id)
                .is_some_and(|value| Arc::ptr_eq(value, &stale))
        );
        assert_eq!(require_open(&mut *held(&stale)?)?.value, 1);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(held(&receipts)?.is_empty());
    }
    assert_eq!(
        close(id, &registry, &receipts, |_, _| Ok("complete".to_owned()))?,
        "complete"
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
#[allow(clippy::panic)]
fn poisoned_session_can_release_without_clearing_its_poison() -> Result<()> {
    let id = SessionId::new(405);
    let drops = Arc::new(AtomicUsize::new(0));
    let (registry, stale) = registered(id, &drops);
    let receipts = Mutex::new(CloseReceipts::new());
    let poisoned = Arc::clone(&stale);
    assert!(
        std::thread::spawn(move || {
            let _guard = poisoned.lock();
            panic!("intentional session-owner failure");
        })
        .join()
        .is_err()
    );
    assert!(stale.is_poisoned());
    assert_eq!(
        close(id, &registry, &receipts, |session, poisoned| {
            assert!(poisoned);
            assert_eq!(session.value, 1);
            Ok("closed-poisoned-owner".to_owned())
        })?,
        "closed-poisoned-owner"
    );
    assert!(stale.is_poisoned());
    let guard = match stale.lock() {
        Ok(_) => return Err(failure("close incorrectly cleared session poison")),
        Err(poison) => poison.into_inner(),
    };
    assert!(guard.is_none());
    drop(guard);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        close(id, &registry, &receipts, |_, _| Err(failure(
            "receipt replay prepared again"
        )))?,
        "closed-poisoned-owner"
    );
    Ok(())
}

#[test]
#[allow(clippy::panic)]
fn poisoned_global_registries_refuse_before_releasing_any_session_resource() -> Result<()> {
    for poison_receipts in [false, true] {
        let id = SessionId::new(406);
        let drops = Arc::new(AtomicUsize::new(0));
        let (registry, stale) = registered(id, &drops);
        let receipts = Arc::new(Mutex::new(CloseReceipts::new()));
        let bad_registry = Arc::clone(&registry);
        let bad_receipts = Arc::clone(&receipts);
        assert!(
            std::thread::spawn(move || {
                if poison_receipts {
                    let _guard = bad_receipts.lock();
                    panic!("intentional receipt-registry failure");
                } else {
                    let _guard = bad_registry.lock();
                    panic!("intentional session-registry failure");
                }
            })
            .join()
            .is_err()
        );
        let prepared = AtomicUsize::new(0);
        let result = close(id, &registry, &receipts, |_, _| {
            prepared.fetch_add(1, Ordering::SeqCst);
            Ok("must-not-publish".to_owned())
        });
        assert!(
            matches!(result, Err(error) if error.code == ErrorCode::InternalInvariantViolation)
        );
        assert_eq!(prepared.load(Ordering::SeqCst), 0);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(require_open(&mut *held(&stale)?)?.value, 1);
    }
    Ok(())
}

#[test]
fn receipt_retention_is_exactly_bounded_and_never_aliases_another_session() -> Result<()> {
    let registry = Mutex::new(SessionRegistry::new());
    let receipts = Mutex::new(CloseReceipts::new());
    let drops = Arc::new(AtomicUsize::new(0));
    let mut stale = Vec::new();
    for sequence in 0..=MAX_CLOSE_RECEIPTS {
        let id = SessionId::new(1_000 + sequence as u128);
        let handle = Arc::new(Mutex::new(Some(OwnedResource {
            value: sequence,
            drops: Arc::clone(&drops),
        })));
        held(&registry)?.insert(id, Arc::clone(&handle));
        stale.push(handle);
        assert_eq!(
            close(id, &registry, &receipts, |session, _| Ok(format!(
                "receipt-{}",
                session.value
            )))?,
            format!("receipt-{sequence}")
        );
        assert!(held(&registry)?.is_empty());
        assert_eq!(
            held(&receipts)?.len(),
            (sequence + 1).min(MAX_CLOSE_RECEIPTS)
        );
    }
    assert_eq!(drops.load(Ordering::SeqCst), MAX_CLOSE_RECEIPTS + 1);
    assert!(matches!(
        close(SessionId::new(1_000), &registry, &receipts, |_, _| Err(failure("evicted receipt prepared"))),
        Err(error) if error.code == ErrorCode::SessionNotFound
    ));
    for sequence in 1..=MAX_CLOSE_RECEIPTS {
        assert_eq!(
            close(
                SessionId::new(1_000 + sequence as u128),
                &registry,
                &receipts,
                |_, _| Err(failure("receipt replay prepared"))
            )?,
            format!("receipt-{sequence}")
        );
    }
    assert!(matches!(
        close(SessionId::new(9_999), &registry, &receipts, |_, _| Err(failure("unknown session prepared"))),
        Err(error) if error.code == ErrorCode::SessionNotFound
    ));
    for handle in &stale {
        assert!(held(handle)?.is_none());
    }
    drop(stale);
    assert_eq!(drops.load(Ordering::SeqCst), MAX_CLOSE_RECEIPTS + 1);
    Ok(())
}

#[test]
fn closing_one_session_does_not_wait_for_or_mutate_a_different_owner() -> Result<()> {
    let first_id = SessionId::new(407);
    let second_id = SessionId::new(408);
    let first_drops = Arc::new(AtomicUsize::new(0));
    let second_drops = Arc::new(AtomicUsize::new(0));
    let (registry, first) = registered(first_id, &first_drops);
    let second = Arc::new(Mutex::new(Some(OwnedResource {
        value: 2,
        drops: Arc::clone(&second_drops),
    })));
    held(&registry)?.insert(second_id, Arc::clone(&second));
    let foreground = held(&first)?;
    let receipts = Arc::new(Mutex::new(CloseReceipts::new()));
    let worker_registry = Arc::clone(&registry);
    let worker_receipts = Arc::clone(&receipts);
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = close(
            second_id,
            &worker_registry,
            &worker_receipts,
            |session, _| {
                assert_eq!(session.value, 2);
                Ok("second-only".to_owned())
            },
        );
        let _ = sender.send(result);
    });
    let result = receiver.recv_timeout(Duration::from_secs(2));
    // Release foreground custody even on a failed assertion/timeout, allowing
    // the supervised test worker to join rather than leaving a detached task.
    drop(foreground);
    worker
        .join()
        .map_err(|_| failure("close worker panicked"))?;
    assert_eq!(
        result.map_err(|_| failure("unrelated session blocked close"))??,
        "second-only"
    );
    assert_eq!(first_drops.load(Ordering::SeqCst), 0);
    assert_eq!(second_drops.load(Ordering::SeqCst), 1);
    assert_eq!(held(&registry)?.len(), 1);
    assert_eq!(require_open(&mut *held(&first)?)?.value, 1);
    assert!(held(&second)?.is_none());
    Ok(())
}

#[cfg(unix)]
#[test]
fn actual_socket_is_closed_before_return_even_when_a_stale_arc_survives() -> Result<()> {
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    let (owned, mut peer) = UnixStream::pair().map_err(|_| failure("fixture socket pair"))?;
    peer.set_read_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| failure("fixture socket timeout"))?;
    let id = SessionId::new(409);
    let stale = Arc::new(Mutex::new(Some(owned)));
    let registry = Mutex::new(BTreeMap::from([(id, Arc::clone(&stale))]));
    let receipts = Mutex::new(CloseReceipts::new());
    assert_eq!(
        close(id, &registry, &receipts, |_, _| Ok(
            "socket-released".to_owned()
        ))?,
        "socket-released"
    );
    let mut byte = [0; 1];
    assert_eq!(
        peer.read(&mut byte)
            .map_err(|_| failure("peer did not observe socket EOF"))?,
        0
    );
    assert!(held(&stale)?.is_none());
    Ok(())
}
