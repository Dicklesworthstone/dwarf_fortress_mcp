//! A source outage interrupts monitoring, not its retained intent. The runtime
//! records this boundary before reconnecting, under the existing session lock.
//! Publication follows the same render -> optional sync -> root-swap transaction
//! as registration. No predicate, native read, deadline extension or game effect.
use super::super::super as watches;
use super::*;

const REASON: &str = "source_gap_requires_fresh_observation";

impl WatchJournalGuard {
    /// Preserve handles, definitions and sample history while retiring any
    /// consecutive-success evidence that would otherwise span a source outage.
    /// Failure to persist/render this boundary must prevent the reconnect.
    pub(crate) fn interrupt_source<F>(
        snapshot: &WorldSnapshot,
        context: &OperationContext,
        publish: F,
    ) -> Result<Value>
    where
        F: FnOnce(Value) -> Result<String>,
    {
        interrupt_in(&WATCHES, snapshot, context, publish)
    }
}

fn interrupt_in<F>(
    storage: &Mutex<Store>,
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    publish: F,
) -> Result<Value>
where
    F: FnOnce(Value) -> Result<String>,
{
    let budget = watches::counts::EvaluationBudget::new(context.budget.max_wall_millis);
    authorize(snapshot, context)?;
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    let mut store = watches::lock(storage)?;
    let mut candidate = Store {
        serial: store.serial,
        entries: store.entries.clone(),
    };
    let mut changed = 0usize;
    let mut pending = 0usize;
    let mut terminal = 0usize;
    let mut records = Vec::new();
    for ((session, _), watch) in &mut candidate.entries {
        if *session != context.session_id {
            continue;
        }
        budget.check()?;
        if records.len() >= MAX_PER_SESSION {
            return Err(failure(
                ErrorCode::InternalInvariantViolation,
                "source interruption exceeds per-session watch retention",
            ));
        }
        if watch.status.terminal() {
            terminal += 1;
        } else {
            let previous = watch.last_seen;
            let incompatible = context.anchor.fortress_id != watch.created_at.fortress_id
                || context.anchor.cursor.epoch != watch.created_at.cursor.epoch
                || context.anchor.tick < previous.tick
                || context.anchor.cursor.sequence < previous.cursor.sequence
                || (context.anchor.cursor == previous.cursor && context.anchor != previous);
            let expired = context.anchor.tick.0 >= watch.definition.deadline_tick;
            let already_interrupted = watch.status == Status::BlockedUnknown
                && watch.last_seen == context.anchor
                && watch.streak == 0
                && watch.evaluation.get("reason").and_then(Value::as_str) == Some(REASON);
            if incompatible || expired || !already_interrupted {
                let prior = watch.evidence_digest;
                watch.status = if incompatible {
                    Status::Invalidated
                } else if expired {
                    Status::Expired
                } else {
                    Status::BlockedUnknown
                };
                watch.streak = 0;
                watch.last_seen = context.anchor;
                // Retain the last actual sample tick and count. Declaring an
                // outage is not a sample and does not renew the original cadence.
                watch.evaluation = json!({"reason":if incompatible {
                    "source_gap_observation_identity_changed"
                } else if expired { "deadline_reached_before_source_recovery" } else { REASON },
                    "previous_anchor":anchor(previous),"prior_evidence_digest":prior.to_string(),
                    "condition":"unknown","failure_condition":"unknown","sample_due":false,
                    "predicate_evaluated":false,"continuous_between_observations":false});
                watch.seal()?;
                changed += 1;
            }
            if watch.status.terminal() {
                terminal += 1;
            } else {
                pending += 1;
            }
        }
        records.push(json!({"watch":watch.handle,"status":watch.status.text(),
            "evidence_digest":watch.evidence_digest.to_string()}));
    }
    let info = json!({"basis":anchor(context.anchor),"changed_watches":changed,
        "pending_watches":pending,"terminal_watches":terminal,"records":records,
        "handles_preserved":true,"definitions_preserved":true,"deadlines_extended":false,
        "samples_added":0,"predicates_evaluated":false,"continuous_during_outage":false});
    let mut value = watches::payload(context, "source_gap");
    value["source_gap"] = info.clone();
    watches::publish_work(&candidate, context, value, |value| {
        let encoded = publish(value)?;
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        budget.check()?;
        Ok(encoded)
    })?;
    // Nothing fallible follows the optional checkpoint sync.
    *store = candidate;
    Ok(info)
}

#[cfg(test)]
#[path = "watch_source_gap_tests.rs"]
mod tests;
