use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct OwnedConnection(Arc<AtomicUsize>);

impl Drop for OwnedConnection {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn retained_handle_loses_connection_and_access_before_close_returns() -> Result<()> {
    let id = SessionId::new(17);
    let drops = Arc::new(AtomicUsize::new(0));
    let handle = Arc::new(Mutex::new(Some(OwnedConnection(Arc::clone(&drops)))));
    let registry = Mutex::new(BTreeMap::from([(id, Arc::clone(&handle))]));
    let receipts = Mutex::new(VecDeque::new());
    let out = close(id, &registry, &receipts, |_, poisoned| {
        assert!(!poisoned);
        Ok("complete close receipt".to_owned())
    })?;
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(lock(&registry)?.is_empty());
    assert!(matches!(
        require_open(&mut *lock(&handle)?),
        Err(failure) if failure.code == ErrorCode::SessionNotFound
    ));
    assert_eq!(
        close(id, &registry, &receipts, |_, _| {
            Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "repeat must not prepare another close",
            ))
        })?,
        out
    );
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn failed_complete_receipt_preflight_preserves_every_owned_resource() -> Result<()> {
    let id = SessionId::new(18);
    let drops = Arc::new(AtomicUsize::new(0));
    let handle = Arc::new(Mutex::new(Some(OwnedConnection(Arc::clone(&drops)))));
    let registry = Mutex::new(BTreeMap::from([(id, Arc::clone(&handle))]));
    let receipts = Mutex::new(VecDeque::new());
    let result = close(id, &registry, &receipts, |_, _| {
        Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "complete receipt does not fit",
        ))
    });
    assert!(matches!(result, Err(failure) if failure.code == ErrorCode::BudgetExceeded));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(lock(&registry)?.len(), 1);
    assert!(require_open(&mut *lock(&handle)?).is_ok());
    assert!(lock(&receipts)?.is_empty());
    Ok(())
}

#[test]
fn teardown_selectors_are_explicit_and_do_not_reinterpret_ordinary_cancel() -> Result<()> {
    assert!(session_close_requested(Some("session"), None, None, None)?);
    assert!(session_close_requested(Some("session"), None, None, Some(true))?);
    assert!(!session_close_requested(None, Some("legacy"), Some("legacy"), None)?);
    for (scope, mode, legacy, discard) in [
        (Some("session"), Some(""), None, None),
        (Some("session"), None, Some(""), None),
        (Some("session"), Some(""), Some(""), Some(true)),
        (Some("session"), Some("action"), None, None),
        (Some("session"), None, Some("action"), None),
        (Some("goal"), None, None, None),
        (Some(""), None, None, None),
        (None, None, None, Some(false)),
        (None, None, None, Some(true)),
    ] {
        assert!(matches!(
            session_close_requested(scope, mode, legacy, discard),
            Err(failure) if failure.code == ErrorCode::InvalidRequest
        ));
    }
    Ok(())
}
