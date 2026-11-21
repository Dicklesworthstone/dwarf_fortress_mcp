//! Release an operations session without reading or changing game state or files.
//! Foreground calls finish before teardown acquires the session. A process-scoped
//! handle establishes ownership; expired read authority does not trap resources.
use super::*;
use std::collections::VecDeque;

const MAX_CLOSE_RECEIPTS: usize = 32;
static CLOSED: LazyLock<Mutex<VecDeque<(SessionId, String)>>> =
    LazyLock::new(|| Mutex::new(VecDeque::new()));

struct ClosedSource;
impl OperationsSource for ClosedSource {
    fn read(&mut self, _: Duration) -> Result<LiveOperationsObservation> {
        Err(error(
            ErrorCode::SessionNotFound,
            "operations session is closed",
        ))
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
                "operations session is not open and no recent close receipt is retained",
            )
        })
}

fn render(session: &OperationsSession, poisoned: bool, released: Value) -> Result<String> {
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
        .continuity(
            ContinuityStatus::Partial,
            None,
            Some(json!({"reason":"session_closed_without_game_observation"})),
            None,
        )
        .briefing(json!({"session_closed":true,"runtime_admitted":false,
            "mutation_admissible":false,"game_state_accessed":false,
            "active_work_scope":"released_process_session_only"}))
        .coverage(
            json!({"status":"partial","complete_domains":["session_resource_release"],
            "omitted_domains":["current_game_state","game_effect_outcomes"]}),
        )
        .attach(value);
    let maximum = session
        .budget
        .max_bytes
        .min(u64::from(session.budget.max_output_tokens) * 4);
    if out.len() as u64 > maximum {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "session close receipt does not fit; no resources were released",
        ));
    }
    Ok(out)
}

fn close(raw: Option<String>, discard_process_local: bool) -> Result<String> {
    let id = parse_session_id(raw.clone())?;
    let handle = match resolve(raw) {
        Ok(handle) => handle,
        Err(e) if e.code == ErrorCode::SessionNotFound => return receipt(id),
        Err(e) => return Err(e),
    };
    // Only teardown recovers a poisoned guard. Never clear poisoning, sample
    // watches, or query partially published state left by an unwinding request.
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
            "operations session ownership changed before close",
        ));
    }
    // Consent and complete response rendering precede all changes. The semantic
    // layer retains its ordinary HISTORY -> WATCHES -> JOURNALS lock order.
    let out = semantic_query::release_session_resources(id, discard_process_local, |released| {
        render(&session, poisoned, released)
    })?;
    // No fallible operation follows release. Replacing the source closes its
    // connection and returns the paged slot even if another call retains an Arc.
    // That waiter is rejected by require_open() before it can access old state.
    session.source = Box::new(ClosedSource);
    session.journal = None;
    session.state = LiveOperationsState::with_profile(session.state.profile());
    session.grants.clear();
    session._slot.release();
    sessions.remove(&id);
    receipts.push_back((id, out.clone()));
    while receipts.len() > MAX_CLOSE_RECEIPTS {
        receipts.pop_front();
    }
    Ok(out)
}

pub(super) fn cancel(raw: Option<String>, scope: Option<String>, discard: Option<bool>) -> String {
    let result = match scope.as_deref() {
        Some("session") => close(raw, discard.unwrap_or(false)),
        None if discard.is_none() => return no_effect(raw, "fortress.cancel"),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "session teardown requires scope=session; ordinary game-effect cancellation remains unavailable",
        )),
    };
    // Refusals contain no cached game facts or implicit watch sampling, even for
    // damaged sources or journals and expired or revoked Query grants.
    match result {
        Ok(out) => out,
        Err(e) => failure_packet(None, None, "fortress.cancel", &e),
    }
}
