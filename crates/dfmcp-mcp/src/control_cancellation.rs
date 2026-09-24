//! Coordinator-only cancellation. No connection, credential, token or native RPC
//! is needed here. A prepared key is retired, never erased or made reusable.
use super::*;
use std::time::Instant;

pub(super) fn cancel<S: EffectJournalStorage>(
    journal: &mut ControlEffectJournal<S>,
    context: &OperationContext,
    key: &str,
    plan: Digest32,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> Result<Value> {
    context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
    let started = Instant::now();
    let bytes = max_bytes.unwrap_or(context.budget.max_bytes);
    let tokens = max_output_tokens.unwrap_or(context.budget.max_output_tokens);
    if bytes == 0
        || bytes > context.budget.max_bytes
        || tokens == 0
        || tokens > context.budget.max_output_tokens
    {
        return Err(err(
            ErrorCode::BudgetExceeded,
            "cancellation response budgets must be positive and only narrow the session limits",
        ));
    }
    let maximum = bytes.min(u64::from(tokens) * 4);
    let mut metadata = journal_json(journal);
    journal.cancel_prepared_with(key, plan, context, |record, length, replayed| {
        if !replayed {
            metadata["head"] = json!(record.record_digest.to_string());
            metadata["transitions"] = json!(record.transition_number);
            metadata["retained_bytes"] = json!(length);
        }
        let value = json!({"schema":"dfmcp.control_cancellation/1","ok":true,
            "session_id":context.session_id.to_string(),"cancelled":true,
            "scope":"this_control_journal_before_dispatch","replayed":replayed,
            "coordinator_dispatch_prevented":true,"same_key_reusable":false,
            "native_cancellation_performed":false,"mutation_dispatched":false,
            "global_effect_absence_proven":false,"current_freshness_proven":false,
            "reconciliation_performed":false,"effect":record_json(record),
            "durable_effect_journal":metadata,
            "response_budget":{"max_bytes":maximum,"max_output_tokens":tokens,
                "token_estimate":"ceil_utf8_bytes_div_4","includes_agent_turn":true}});
        if packet("fortress.cancel", value.clone(), false).len() as u64 > maximum {
            return Err(err(
                ErrorCode::BudgetExceeded,
                "complete cancellation acknowledgement does not fit; no cancellation was written",
            ));
        }
        if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
            return Err(err(
                ErrorCode::BudgetExceeded,
                "cancellation deadline exhausted before journal publication",
            ));
        }
        Ok(value)
    })
    // No fallible post-sync rendering. with_session attaches exactly the packet
    // already checked above. File sync is synchronous, not hard-preemptible.
}

pub(super) fn explanation<S: EffectJournalStorage>(
    journal: &ControlEffectJournal<S>,
    record: &DurablePauseRecord,
) -> Value {
    json!({"ok":true,"effect":record_json(record),"cancelled":true,
        "commit_permitted":false,"reconciliation_performed":false,
        "native_cancellation_performed":false,"mutation_dispatched":false,
        "scope":"this_control_journal_before_dispatch","same_key_reusable":false,
        "global_effect_absence_proven":false,"current_freshness_proven":false,
        "durable_effect_journal":journal_json(journal)})
}

#[cfg(all(test, unix))]
#[path = "control_cancellation_tests.rs"]
mod tests;
