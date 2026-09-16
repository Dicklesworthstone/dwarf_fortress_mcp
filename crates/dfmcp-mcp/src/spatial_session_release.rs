//! Explicit release of a read-only spatial session, including fenced sources.
//! Session ownership is resolved using the same process-scoped handle as other
//! calls. Teardown reveals no game facts and never revives expired read grants.
//! A close waits for the session's current foreground call; it does not abort I/O.
use super::*;
use std::collections::VecDeque;

const MAX_CLOSE_RECEIPTS: usize = 32;
static CLOSED: LazyLock<Mutex<VecDeque<(SessionId, String)>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

pub(super) struct Slot { held: bool }
impl Slot {
    pub(super) fn reserve() -> Result<Self> {
        SLOTS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| (n < 2).then_some(n + 1))
            .map_err(|_| error(ErrorCode::BudgetExceeded, "spatial/1.8 retains at most two sessions"))?;
        Ok(Self { held: true })
    }
    fn release(&mut self) {
        if self.held {
            self.held = false;
            SLOTS.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
impl Drop for Slot { fn drop(&mut self) { self.release(); } }

struct ClosedSource { archive: bool }
impl Source for ClosedSource {
    fn read(&mut self, _: Duration) -> Result<LiveSpatialCitizenObservation> {
        Err(error(ErrorCode::SessionNotFound, "spatial session is closed"))
    }
    fn poisoned(&self) -> bool { true }
    fn fence(&mut self) {}
    fn pages(&self) -> u32 { 0 }
    fn archive_only(&self) -> bool { self.archive }
    fn closed(&self) -> bool { true }
}

fn receipt(id: SessionId) -> Result<String> {
    lock(&CLOSED)?.iter().find(|(session, _)| *session == id).map(|(_, out)| out.clone())
        .ok_or_else(|| error(ErrorCode::SessionNotFound,
            "spatial session is not open and no recent close receipt is retained"))
}

fn render(session: &Session, poisoned: bool, released: Value) -> Result<String> {
    let value = json!({"ok":true,"scope":"session","closed":true,
        "session_id":session.id.to_string(),"release_only":true,
        "process_local_resources":released,"session_mutex_poisoned":poisoned,
        "observation_journal_present":session.journal.is_some(),
        "observation_journal_modified":false,"native_captures":0,
        "game_effects_cancelled":false,"mutation_dispatched":false,
        "game_state_observed":false,"close_receipt_retention":MAX_CLOSE_RECEIPTS,
        "receipt_durable":false,"reopen_requires_new_session":true});
    let out = AgentTurnBuilder::new("fortress.cancel", AgentPhase::Inspect)
        .session_id(session.id.to_string())
        .continuity(ContinuityStatus::Partial, None,
            Some(json!({"reason":"session_closed_without_game_observation"})), None)
        .briefing(json!({"session_closed":true,"runtime_admitted":false,
            "mutation_admissible":false,"game_state_accessed":false,
            "active_work_scope":"released_process_session_only",
            "persisted_monitoring_may_remain":true}))
        .coverage(json!({"status":"partial","complete_domains":["session_resource_release"],
            "omitted_domains":["current_game_state","durable_watch_evidence","game_effect_outcomes"]}))
        .attach(value);
    let maximum = session.budget.max_bytes.min(u64::from(session.budget.max_output_tokens) * 4);
    if out.len() as u64 > maximum {
        return Err(error(ErrorCode::BudgetExceeded, "session close receipt does not fit; no resources were released"));
    }
    Ok(out)
}

pub(super) fn close(raw: Option<String>, discard_process_local: bool) -> Result<String> {
    let id = parse_session_id(raw.clone())?;
    let handle = match resolve(raw) {
        Ok(handle) => handle,
        Err(e) if e.code == ErrorCode::SessionNotFound => return receipt(id),
        Err(e) => return Err(e),
    };
    // Recover the guard only for release. Do not clear poisoning or execute a
    // query against possibly incomplete state after an unwinding request.
    let (mut session, poisoned) = match handle.lock() {
        Ok(guard) => (guard, false),
        Err(poison) => (poison.into_inner(), true),
    };
    if session.source.closed() { return receipt(id); }
    let mut sessions = lock(&SESSIONS)?;
    let mut receipts = lock(&CLOSED)?;
    if !sessions.get(&id).is_some_and(|current| Arc::ptr_eq(current, &handle)) {
        return Err(error(ErrorCode::SessionNotFound, "spatial session ownership changed before close"));
    }
    // The semantic layer locks baseline/watch registries in their normal order,
    // validates explicit consent for volatile records, and renders before release.
    // Neither source health nor expired Query authority prevents resource cleanup.
    let out = semantic_query::release_session_resources(id, discard_process_local,
        |released| render(&session, poisoned, released))?;
    // No fallible operation after this point. Already-resolved waiters still own
    // an Arc, but context()/refresh() reject this ClosedSource before reading data.
    let archive = session.source.archive_only();
    session.source = Box::new(ClosedSource { archive });
    session._watch_journal = None;
    session.journal = None;
    session.state = LiveSpatialCitizenState::default();
    session.grants.clear();
    session._slot.release();
    sessions.remove(&id);
    // A long-lived session may close after many newer sessions. Retain its new
    // receipt by close order so it is not immediately evicted by an older ID.
    receipts.push_back((id, out.clone()));
    while receipts.len() > MAX_CLOSE_RECEIPTS { receipts.pop_front(); }
    Ok(out)
}

pub(super) fn cancel(raw: Option<String>, scope: Option<String>, discard: Option<bool>) -> String {
    let result = match scope.as_deref() {
        Some("session") => close(raw, discard.unwrap_or(false)),
        None if discard.is_none() => return no_effect(raw, "fortress.cancel"),
        _ => Err(error(ErrorCode::InvalidRequest,
            "session teardown requires scope=session; ordinary game-effect cancellation remains unavailable")),
    };
    // A cleanup refusal must not itself sample watches, access a damaged journal
    // or reveal cached world facts through the usual operational error briefing.
    match result { Ok(out) => out, Err(e) => failure(None, None, "fortress.cancel", &e) }
}

#[cfg(all(test, unix))]
#[path = "spatial_session_release_tests.rs"]
mod tests;
