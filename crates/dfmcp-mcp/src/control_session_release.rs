//! Resource teardown, not effect cancellation. No bridge method or journal write
//! is permitted on this path. The owning session lock drains its foreground call.
use super::*;
use std::collections::VecDeque;

const RETAINED_CLOSE_RECEIPTS: usize = 32;
struct Receipt {
    id: SessionId,
    response: String,
    budget: WorkBudget,
}
static CLOSED: LazyLock<Mutex<VecDeque<Receipt>>> = LazyLock::new(|| {
    Mutex::new(VecDeque::with_capacity(RETAINED_CLOSE_RECEIPTS))
});

fn ceiling(budget: WorkBudget, bytes: Option<u64>, tokens: Option<u32>) -> Result<u64> {
    let bytes = bytes.unwrap_or(budget.max_bytes);
    let tokens = tokens.unwrap_or(budget.max_output_tokens);
    if bytes == 0 || bytes > budget.max_bytes || tokens == 0 || tokens > budget.max_output_tokens {
        return Err(err(ErrorCode::BudgetExceeded, "close budgets must be positive and only narrow session limits"));
    }
    Ok(bytes.min(u64::from(tokens) * 4))
}
fn replay(receipts: &VecDeque<Receipt>, id: SessionId,
    bytes: Option<u64>, tokens: Option<u32>) -> Result<String> {
    let receipt = receipts.iter().find(|r| r.id == id)
        .ok_or_else(|| err(ErrorCode::SessionNotFound, "control session is absent and no close receipt is retained"))?;
    if receipt.response.len() as u64 > ceiling(receipt.budget, bytes, tokens)? {
        return Err(err(ErrorCode::BudgetExceeded, "retained control close receipt exceeds the requested output budget"));
    }
    Ok(receipt.response.clone())
}

fn response(id: SessionId) -> String {
    // No cached game or journal facts are disclosed: teardown remains available
    // after grant expiry, counter exhaustion, a fenced journal or a source panic.
    let mut out: Value = serde_json::json!({
        "agent_turn": {
            "operation": "fortress.cancel", "phase": "act",
            "briefing": {"runtime": "unadmitted_development", "bridge_protocol": "1.7",
                "runtime_admitted": false, "mutation_admissible": false,
                "development_mutation_enabled": false, "supported_effects": []},
            "coverage": {"status": "partial", "complete_domains": ["control_session_resource_release"],
                "omitted_domains": ["live_world_state", "durable_effect_outcomes"]},
            "uncertainty": [{"code": "close_is_not_effect_cancellation",
                "detail": "Prepared effects remain prepared; unresolved attempts still require reconciliation after reopening."}]
        },
        "result": {"ok": true, "mode": "session_closed", "session_id": id.to_string(),
            "connection_released": true, "journal_lock_released": true, "session_capacity_released": true,
            "journal_written": false, "journal_repaired": false, "journal_deleted": false,
            "prepared_effects_cancelled": false, "native_cancellation_performed": false,
            "mutation_dispatched": false, "reconciliation_performed": false,
            "current_freshness_proven": false, "safe_to_retry_same_effect": false,
            "reopen_required": true, "effects_must_be_rediscovered_after_reopen": true,
            "receipt_scope": "this_process_last_32_completed_closes"}
    });
    // Explicit null avoids implying a historical effect was just resolved.
    out["result"]["prior_effects_requiring_reconciliation"] = Value::Null;
    out.to_string()
}

/// Destructively take the session only after the complete response and both
/// registries are ready. Old Arc holders see None and cannot perform more work.
/// Lock order: per-session -> SESSIONS -> CLOSED. Resolution never waits for a
/// per-session lock while holding SESSIONS, and replay never acquires SESSIONS.
pub(super) fn close(raw: Option<String>, bytes: Option<u64>, tokens: Option<u32>) -> Result<String> {
    let raw = raw.ok_or_else(|| err(ErrorCode::InvalidRequest, "session closure requires a control session ID"))?;
    let id = parse_session_id(&raw)?;
    let handle = {
        let sessions = lock(&SESSIONS)?;
        match sessions.get(&id) {
            Some(handle) => handle.clone(),
            None => return replay(&lock(&CLOSED)?, id, bytes, tokens),
        }
    };
    // Recover only enough ownership to DROP a poisoned session. No action,
    // diagnostic, cached outcome or bridge operation runs through this guard.
    let mut owned = match handle.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(session) = owned.as_ref() else {
        return replay(&lock(&CLOSED)?, id, bytes, tokens);
    };
    let budget = session.budget;
    let out = response(id);
    if out.len() as u64 > ceiling(budget, bytes, tokens)? {
        return Err(err(ErrorCode::BudgetExceeded, "complete control close acknowledgement does not fit; session remains open"));
    }
    let receipt = Receipt { id, response: out.clone(), budget };
    let mut sessions = lock(&SESSIONS)?;
    if !sessions.get(&id).is_some_and(|registered| Arc::ptr_eq(registered, &handle)) {
        return Err(err(ErrorCode::Conflict, "control session registry changed before release"));
    }
    let mut closed = lock(&CLOSED)?;
    // All fallible validation and allocation of the response precede this point.
    // ControlSession owns connection, journal and Slot in that drop order. The
    // journal is closed before capacity can be reused by another opening.
    drop(owned.take());
    sessions.remove(&id);
    if closed.len() == RETAINED_CLOSE_RECEIPTS { closed.pop_front(); }
    closed.push_back(receipt);
    Ok(out)
}

#[cfg(all(test, unix))]
#[path = "control_session_release_tests.rs"]
mod tests;
