//! Resource-only ownership for live read sessions. Closing takes the complete
//! session out of its cell, so a waiter retaining an Arc cannot retain a native
//! connection or regain access to cached game state after release.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use dfmcp_core::{DfmcpError, ErrorCode, Result, SessionId};

pub(crate) const MAX_CLOSE_RECEIPTS: usize = 32;
pub(crate) type SessionHandle<T> = Arc<Mutex<Option<T>>>;
pub(crate) type SessionRegistry<T> = BTreeMap<SessionId, SessionHandle<T>>;
pub(crate) type CloseReceipts = VecDeque<(SessionId, String)>;

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex.lock().map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "live session release registry or receipt mutex is poisoned",
        )
    })
}

fn closed_error() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::SessionNotFound,
        "live session is closed; open a new session to observe game state",
    )
}

/// Called under the ordinary, non-poison-recovering session guard before any
/// authority, anchor, request counter, adapter or cached presentation is used.
pub(crate) fn require_open<T>(session: &mut Option<T>) -> Result<&mut T> {
    session.as_mut().ok_or_else(closed_error)
}

/// Preserve the old cancellation denial unless resource teardown is explicit.
/// `_mode` was the literal legacy MCP argument, so both spellings are checked.
pub(crate) fn session_close_requested(
    scope: Option<&str>,
    mode: Option<&str>,
    legacy_mode: Option<&str>,
    discard_process_local_work: Option<bool>,
) -> Result<bool> {
    match scope {
        Some("session") if mode.is_none() && legacy_mode.is_none() => Ok(true),
        None if discard_process_local_work.is_none() => Ok(false),
        _ => Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "resource teardown requires scope=session without a mode; ordinary game-effect cancellation remains unavailable",
        )),
    }
}

fn receipt(receipts: &CloseReceipts, id: SessionId) -> Result<String> {
    receipts
        .iter()
        .find(|(session, _)| *session == id)
        .map(|(_, out)| out.clone())
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::SessionNotFound,
                "live session is not open and no recent close receipt is retained",
            )
        })
}

/// Release one owned cell after its foreground operation finishes. `prepare`
/// must fit the complete response and atomically release any shared query work
/// before returning success. Nothing fallible follows that success here.
///
/// Resolution releases the registry guard before waiting for the cell. Close
/// then holds cell -> registry -> receipts while preparing shared cleanup.
/// Only this resource-only operation may recover a poisoned session guard;
/// poisoning in shared registries still refuses before any release.
pub(crate) fn close<T>(
    id: SessionId,
    registry: &Mutex<SessionRegistry<T>>,
    receipts: &Mutex<CloseReceipts>,
    prepare: impl FnOnce(&T, bool) -> Result<String>,
) -> Result<String> {
    let handle = {
        let sessions = lock(registry)?;
        match sessions.get(&id) {
            Some(handle) => Arc::clone(handle),
            None => return receipt(&*lock(receipts)?, id),
        }
    };
    let (mut session, poisoned) = match handle.lock() {
        Ok(guard) => (guard, false),
        Err(poison) => (poison.into_inner(), true),
    };
    let mut sessions = lock(registry)?;
    let mut receipts = lock(receipts)?;
    match sessions.get(&id) {
        None if session.is_none() => return receipt(&receipts, id),
        Some(current) if Arc::ptr_eq(current, &handle) && session.is_some() => {}
        _ => {
            return Err(DfmcpError::new(
                ErrorCode::SessionNotFound,
                "live session ownership changed before resource release",
            ));
        }
    }
    let open = require_open(&mut session)?;
    let out = prepare(open, poisoned)?;
    // Taking drops the concrete adapter, socket, credentials, grants and cached
    // world even when foreground waiters still hold a cloned session Arc.
    drop(session.take());
    sessions.remove(&id);
    receipts.push_back((id, out.clone()));
    while receipts.len() > MAX_CLOSE_RECEIPTS {
        receipts.pop_front();
    }
    Ok(out)
}

#[cfg(test)]
#[path = "read_session_release_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "read_session_release_validation_tests.rs"]
mod validation_tests;
