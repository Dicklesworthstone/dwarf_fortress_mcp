//! Bounded response reservation and live read-only transport for fortress.wait.
use super::*;
use dfmcp_adapter::pause_reconciliation::{
    self, PauseReconciliationSource, ReconciliationBatch, ReconciliationItem,
};
use std::time::Instant;

impl PauseReconciliationSource for ControlConnection {
    fn query_effect(
        &mut self,
        record: &DurablePauseRecord,
        remaining: Duration,
        context: &OperationContext,
    ) -> Result<PauseEffect> {
        context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
        let started = Instant::now();
        if self.client.poisoned() {
            self.client = ControlRpcClient::connect(
                self.endpoint,
                self.token.clone(),
                self.nonce.clone(),
                remaining,
            )?;
        }
        let remaining = remaining
            .checked_sub(started.elapsed())
            .filter(|v| *v >= Duration::from_millis(1))
            .ok_or_else(|| {
                err(
                    ErrorCode::BudgetExceeded,
                    "reconciliation deadline exhausted during reconnect",
                )
            })?;
        self.client.reset_deadline(remaining)?;
        let result = self
            .client
            .query_pause(&record.idempotency_key, record.plan_digest);
        if result.is_err() {
            self.client.fence();
        }
        result
    }
}

fn ceiling(context: &OperationContext, bytes: Option<u64>, tokens: Option<u32>) -> Result<usize> {
    let bytes = bytes.unwrap_or(context.budget.max_bytes);
    let tokens = tokens.unwrap_or(context.budget.max_output_tokens);
    if bytes == 0
        || bytes > context.budget.max_bytes
        || tokens == 0
        || tokens > context.budget.max_output_tokens
    {
        return Err(err(
            ErrorCode::BudgetExceeded,
            "wait response budgets must be positive and only narrow session limits",
        ));
    }
    usize::try_from(bytes.min(u64::from(tokens) * 4)).map_err(|_| {
        err(
            ErrorCode::BudgetExceeded,
            "wait response byte limit cannot be represented",
        )
    })
}

fn payload(batch: &ReconciliationBatch, journal: Value, maximum: usize) -> Value {
    let queried = batch.items.iter().filter(|v| v.queried).count();
    let deferred = batch.items.iter().filter(|v| v.deferred).count();
    let unresolved = batch
        .items
        .iter()
        .filter(|v| v.record.state.reconciliation_required())
        .count();
    let items: Vec<_> = batch
        .items
        .iter()
        .map(|item| {
            json!({
        "effect":record_json(&item.record),"queried":item.queried,"deferred":item.deferred,
        "error_code":item.error.map(|e|e.as_str())})
        })
        .collect();
    json!({"ok":batch.stopped.is_none(),"mode":"reconciliation_pass","items":items,
        "selected":batch.items.len(),"queried":queried,"deferred":deferred,
        "unresolved":unresolved,"all_terminal":batch.items.iter().all(|v|v.record.state.terminal()),
        "pass_complete":batch.stopped.is_none(),"stopped_error_code":batch.stopped.map(|e|e.as_str()),
        "journal_head_before":batch.head_before.to_string(),"journal_head_after":batch.head_after.to_string(),
        "mutation_dispatched":false,"safe_to_retry_same_effect":false,"current_freshness_proven":false,
        "durable_effect_journal":journal,"response_budget_bytes":maximum,
        "token_estimation":"ceil_utf8_bytes_div_4"})
}

/// Reserve complete worst-case records and Agent Turn BEFORE any bridge query.
fn reserve(selected: &[DurablePauseRecord], journal: Value, maximum: usize) -> Result<()> {
    let items = selected
        .iter()
        .map(|record| {
            let mut record = record.clone();
            record.state = DurablePauseState::VerifiedNotApplied;
            record.effect_known = false;
            record.effect_applied = false;
            record.observed_paused = Some(false);
            record.observed_game_tick = Some(u64::MAX);
            record.receipt_digest = Some(Digest32::ZERO);
            record.revision = u64::MAX;
            record.transition_number = u64::MAX;
            ReconciliationItem {
                record,
                queried: false,
                deferred: false,
                error: None,
            }
        })
        .collect();
    let sample = ReconciliationBatch {
        items,
        head_before: Digest32::ZERO,
        head_after: Digest32::ZERO,
        stopped: None,
    };
    let mut body = payload(&sample, journal, maximum);
    body["ok"] = json!(false);
    body["all_terminal"] = json!(false);
    body["pass_complete"] = json!(false);
    body["stopped_error_code"] = json!("x".repeat(64));
    if let Some(items) = body["items"].as_array_mut() {
        for item in items {
            item["error_code"] = json!("x".repeat(64));
        }
    }
    for field in [
        "effects",
        "transitions",
        "retained_bytes",
        "repaired_tail_bytes",
    ] {
        body["durable_effect_journal"][field] = json!(u64::MAX);
    }
    body["durable_effect_journal"]["fenced"] = json!(false);
    let length = packet("fortress.wait", body, false)
        .len()
        .saturating_add(256);
    if length > maximum {
        return Err(err(
            ErrorCode::BudgetExceeded,
            "selected effects cannot fit a complete wait response; select fewer keys or increase the response allowance",
        ));
    }
    Ok(())
}

pub(super) fn wait(
    session: &mut ControlSession,
    mut context: OperationContext,
    keys: Vec<String>,
    wall_millis: Option<u64>,
    bytes: Option<u64>,
    tokens: Option<u32>,
) -> Result<Value> {
    let started = Instant::now();
    context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
    session.live()?;
    let millis = wall_millis.unwrap_or(context.budget.max_wall_millis);
    if millis == 0 || millis > context.budget.max_wall_millis {
        return Err(err(
            ErrorCode::BudgetExceeded,
            "wait wall-time must be positive and may only narrow the session budget",
        ));
    }
    let maximum = ceiling(&context, bytes, tokens)?;
    let selected = pause_reconciliation::select_effects(&mut session.journal, &keys, &context)?;
    reserve(&selected, journal_json(&session.journal), maximum)?;
    let remaining = Duration::from_millis(millis)
        .checked_sub(started.elapsed())
        .filter(|v| *v >= Duration::from_millis(1))
        .ok_or_else(|| {
            err(
                ErrorCode::BudgetExceeded,
                "wait deadline exhausted before any bridge query",
            )
        })?;
    context.budget.max_wall_millis = u64::try_from(remaining.as_millis())
        .map_err(|_| err(ErrorCode::BudgetExceeded, "wait deadline overflow"))?;
    let connection = session.connection.as_mut().ok_or_else(|| {
        err(
            ErrorCode::CapabilityDenied,
            "offline recovery cannot reconcile live effects",
        )
    })?;
    let batch =
        pause_reconciliation::reconcile_batch(&mut session.journal, connection, &keys, &context)?;
    let body = payload(&batch, journal_json(&session.journal), maximum);
    if packet("fortress.wait", body.clone(), false).len() > maximum {
        return Err(err(
            ErrorCode::InternalInvariantViolation,
            "wait response exceeded its preflight reservation; inspect the durable journal before further work",
        ));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record(key: String) -> DurablePauseRecord {
        DurablePauseRecord {
            idempotency_key: key,
            plan_digest: Digest32::ZERO,
            desired_paused: true,
            expected_game_tick: 10,
            bridge_generation: 7,
            prepare_token: [1; 16],
            state: DurablePauseState::CommitStarted,
            effect_known: false,
            effect_applied: false,
            observed_paused: None,
            observed_game_tick: None,
            receipt_digest: None,
            revision: 2,
            transition_number: 2,
            previous_digest: Digest32::ZERO,
            record_digest: Digest32::ZERO,
        }
    }
    fn metadata() -> Value {
        json!({"journal_id":Digest32::ZERO.to_string(),"head":Digest32::ZERO.to_string(),
            "effects":1,"transitions":2,"retained_bytes":100,"repaired_tail_bytes":0,
            "fenced":false,"restart_recovery":true,"read_only":false})
    }
    #[test]
    fn reservation_covers_terminal_unknown_failed_and_deferred_shapes() -> Result<()> {
        for key in ["k".to_owned(), "\\\"".repeat(256), "\u{1f332}".repeat(128)] {
            let base = record(key);
            let maximum = 16_384;
            reserve(std::slice::from_ref(&base), metadata(), maximum)?;
            for state in [
                DurablePauseState::Prepared,
                DurablePauseState::CommitStarted,
                DurablePauseState::Indeterminate,
                DurablePauseState::VerifiedApplied,
                DurablePauseState::VerifiedNotApplied,
            ] {
                let mut r = base.clone();
                r.state = state;
                r.revision = u64::MAX;
                r.transition_number = u64::MAX;
                r.observed_paused = Some(false);
                r.observed_game_tick = Some(u64::MAX);
                r.receipt_digest = Some(Digest32::ZERO);
                let batch = ReconciliationBatch {
                    items: vec![ReconciliationItem {
                        record: r,
                        queried: true,
                        deferred: true,
                        error: Some(ErrorCode::InternalInvariantViolation),
                    }],
                    head_before: Digest32::ZERO,
                    head_after: Digest32::ZERO,
                    stopped: Some(ErrorCode::InternalInvariantViolation),
                };
                assert!(
                    packet("fortress.wait", payload(&batch, metadata(), maximum), false).len()
                        <= maximum
                );
            }
        }
        Ok(())
    }
    #[test]
    fn tiny_response_is_refused_before_reconciliation() {
        assert!(matches!(reserve(&[record("k".to_owned())],metadata(),128),
            Err(e) if e.code==ErrorCode::BudgetExceeded));
    }
}
