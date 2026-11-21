//! Release a read-only jobs session without native reads or game effects.
//! A valid process-scoped handle owns these resources even when read grants
//! expire or the native connection has been fenced after a failed observation.
use super::*;
use crate::read_session_release::MAX_CLOSE_RECEIPTS;
use std::collections::VecDeque;

static CLOSED: LazyLock<Mutex<VecDeque<(SessionId, String)>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

struct ClosedSource;
impl JobSource for ClosedSource {
    fn read(&mut self, _: Duration) -> Result<LiveJobObservation> {
        Err(error(ErrorCode::SessionNotFound, "jobs session is closed"))
    }
    fn poisoned(&self) -> bool {
        true
    }
    fn fence(&mut self) {}
    fn closed(&self) -> bool {
        true
    }
}

fn receipt(id: SessionId) -> Result<String> {
    lock(&CLOSED)?
        .iter()
        .find(|(session, _)| *session == id)
        .map(|(_, out)| out.clone())
        .ok_or_else(|| {
            error(
                ErrorCode::SessionNotFound,
                "jobs session is not open and no recent close receipt is retained",
            )
        })
}

fn close(raw: Option<String>, discard_process_local: bool) -> Result<String> {
    let id = parse_session_id(raw.clone())?;
    let handle = match resolve(raw) {
        Ok(handle) => handle,
        Err(e) if e.code == ErrorCode::SessionNotFound => return receipt(id),
        Err(e) => return Err(e),
    };
    // Only teardown recovers a poisoned guard. Never clear poison, sample
    // watches, or query partially published state from an unwinding request.
    let (mut session, poisoned) = match handle.lock() {
        Ok(guard) => (guard, false),
        Err(poison) => (poison.into_inner(), true),
    };
    if session.source.closed() {
        return receipt(id);
    }
    let mut sessions = lock(&SESSIONS)?;
    let mut receipts = lock(&CLOSED)?;
    if !sessions
        .get(&id)
        .is_some_and(|current| Arc::ptr_eq(current, &handle))
    {
        return Err(error(
            ErrorCode::SessionNotFound,
            "jobs session ownership changed before close",
        ));
    }
    // Consent and complete response rendering precede every change. Shared
    // baselines and watches retain their HISTORY -> WATCHES -> JOURNALS order.
    let out = semantic_query::release_session_resources(id, discard_process_local, |released| {
        crate::read_session_release_packet::render(
            session.id,
            session.budget,
            "jobs/1.2",
            true,
            poisoned,
            released,
        )
    })?;
    // No fallible operation follows release. Drop the native source and return
    // its capacity now, including when another caller retains a stale Arc.
    // Every stale session entrypoint is fenced by require_open().
    session.source = Box::new(ClosedSource);
    session.state = LiveJobsState::default();
    session.grants.clear();
    session._slot.release();
    sessions.remove(&id);
    receipts.push_back((id, out.clone()));
    while receipts.len() > MAX_CLOSE_RECEIPTS {
        receipts.pop_front();
    }
    Ok(out)
}

pub(super) fn cancel(
    raw: Option<String>,
    scope: Option<String>,
    discard: Option<bool>,
    mode: Option<String>,
    legacy_mode: Option<String>,
) -> String {
    let result = match crate::read_session_release::session_close_requested(
        scope.as_deref(),
        mode.as_deref(),
        legacy_mode.as_deref(),
        discard,
    ) {
        Ok(true) => close(raw, discard.unwrap_or(false)),
        Ok(false) => return no_effect(raw, "fortress.cancel"),
        Err(failure) => Err(failure),
    };
    // Refusals carry no cached game facts, perform no watch sampling, and do
    // not need read authority or a functioning source to release resources.
    match result {
        Ok(out) => out,
        Err(e) => error_packet(None, None, "fortress.cancel", &e),
    }
}
