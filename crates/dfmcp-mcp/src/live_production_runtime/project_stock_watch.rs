//! Historical no-order stock ownership authenticated by the resource-project
//! journal. A checkpoint is identity and rejection history, never a source read,
//! unfinished stability streak, current proof or native dispatch permission.
use super::*;
use dfmcp_adapter::semantic_work_orders::{
    StockTargetWatch, StockTargetsWatch, StockWatchCheckpointTerms,
};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "dfmcp.project-stock-watch/1";
const MAX_CHECKPOINT_BYTES: usize = 16 * 1024;
const MAX_WATCH_BYTES: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum OriginalRequest {
    Single(StockTargetRequest),
    Portfolio(StockTargetsRequest),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Checkpoint {
    schema: String,
    request: OriginalRequest,
    checkpoint_hex: String,
    cancelled: bool,
}

fn encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 15)]));
    }
    text
}

fn decode(text: &str) -> Result<Vec<u8>> {
    if text.is_empty() || text.len() > MAX_WATCH_BYTES * 2 || text.len() % 2 != 0 {
        return Err(invalid(
            "project stock-watch checkpoint exceeds its bounded encoding",
        ));
    }
    let digit = |value: u8| match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    };
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = digit(pair[0]).ok_or_else(|| invalid("noncanonical stock-watch hex"))?;
            let low = digit(pair[1]).ok_or_else(|| invalid("noncanonical stock-watch hex"))?;
            Ok((high << 4) | low)
        })
        .collect()
}

impl Checkpoint {
    fn scalar(goal: &StockTargetNoWork) -> Self {
        Self {
            schema: SCHEMA.into(),
            request: OriginalRequest::Single(goal.request.clone()),
            checkpoint_hex: encode(&goal.monitor.checkpoint_bytes()),
            cancelled: goal.cancelled,
        }
    }

    fn portfolio(goal: &StockTargetsNoWork) -> Self {
        Self {
            schema: SCHEMA.into(),
            request: OriginalRequest::Portfolio(goal.request.clone()),
            checkpoint_hex: encode(&goal.monitor.checkpoint_bytes()),
            cancelled: goal.cancelled,
        }
    }

    pub(super) fn from_json(value: &Value) -> Result<Self> {
        let bytes = serde_json::to_vec(value)
            .map_err(|_| invalid("project stock-watch custody is not serializable"))?;
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(exhausted("project stock-watch custody exceeds 16 KiB"));
        }
        let checkpoint: Self = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("invalid closed project stock-watch checkpoint"))?;
        if checkpoint.schema != SCHEMA || checkpoint.to_json() != *value {
            return Err(invalid("project stock-watch custody is not canonical"));
        }
        checkpoint.terms()?;
        Ok(checkpoint)
    }

    pub(super) fn to_json(&self) -> Value {
        json!(self)
    }

    /// Bind the complete immutable request to the adapter's validated original.
    /// Native qualification happened before the parent first acknowledged these
    /// bytes; decoding them alone cannot issue another assessment or sample.
    pub(super) fn terms(&self) -> Result<StockWatchCheckpointTerms> {
        let bytes = decode(&self.checkpoint_hex)?;
        let (terms, targets, timing) = match &self.request {
            OriginalRequest::Single(request) => {
                request.validate()?;
                (
                    StockTargetWatch::checkpoint_terms(&bytes)?,
                    vec![(
                        request.recipe.as_str(),
                        request.minimum_stack_units,
                        request.max_order_amount,
                    )],
                    (
                        request.deadline_tick,
                        request.poll_interval_ticks,
                        request.stable_samples,
                    ),
                )
            }
            OriginalRequest::Portfolio(request) => {
                request.validate()?;
                (
                    StockTargetsWatch::checkpoint_terms(&bytes)?,
                    request
                        .targets
                        .iter()
                        .map(|target| {
                            (
                                target.recipe.as_str(),
                                target.minimum_stack_units,
                                target.max_order_amount,
                            )
                        })
                        .collect(),
                    (
                        request.deadline_tick,
                        request.poll_interval_ticks,
                        request.stable_samples,
                    ),
                )
            }
        };
        if (
            terms.deadline_tick.get(),
            terms.poll_interval_ticks,
            terms.stable_samples,
        ) != timing
            || terms.targets.len() != targets.len()
            || terms
                .targets
                .iter()
                .zip(targets)
                .any(|((recipe, minimum, maximum), expected)| {
                    (recipe.as_str(), *minimum, *maximum) != expected
                })
        {
            return Err(denied(
                "project stock watch changed its original target or timing",
            ));
        }
        Ok(terms)
    }

    pub(super) fn validate_goal(&self, goal: &ProjectStockGoal) -> Result<()> {
        self.terms()?;
        match &self.request {
            OriginalRequest::Single(request) => goal.validate_request(request),
            OriginalRequest::Portfolio(request) => goal.validate_targets_request(request),
        }
    }

    pub(super) fn require_unfinished(&self) -> Result<()> {
        let terms = self.terms()?;
        if self.cancelled
            || terms.achieved_at.is_some()
            || terms.expired
            || terms.fenced
            || terms.high_tick >= terms.deadline_tick
        {
            return Err(denied(
                "terminal stock-watch history cannot authorize first production",
            ));
        }
        Ok(())
    }

    fn require_initial(&self) -> Result<()> {
        self.require_unfinished()?;
        let terms = self.terms()?;
        if terms.high_tick != terms.original_anchor.tick {
            return Err(denied(
                "a stock watch must be acknowledged before later samples",
            ));
        }
        Ok(())
    }

    fn successor(&self, current: &Self) -> Result<()> {
        self.terms()?;
        current.terms()?;
        if self.request != current.request || (self.cancelled && !current.cancelled) {
            return Err(denied(
                "project stock watch replaced its request or erased cancellation",
            ));
        }
        let previous = decode(&self.checkpoint_hex)?;
        let next = decode(&current.checkpoint_hex)?;
        match &self.request {
            OriginalRequest::Single(_) => {
                StockTargetWatch::verify_checkpoint_continuity(&previous, &next)
            }
            OriginalRequest::Portfolio(_) => {
                StockTargetsWatch::verify_checkpoint_continuity(&previous, &next)
            }
        }
    }
}

/// Only the actual parent journal calls this admission/replay check. Its
/// private files and independent ACK supply custody; the JSON supplies no trust.
pub(crate) fn validate_parent_reference(
    reference: &Value,
    goal: &ProjectStockGoal,
    fortress: &str,
    initial_tick: u64,
    retained_tick: u64,
) -> Result<()> {
    let Some(checkpoint) = from_reference(reference)? else {
        return Ok(());
    };
    checkpoint.validate_goal(goal)?;
    let terms = checkpoint.terms()?;
    if terms.original_anchor.fortress_id.get().to_string() != fortress
        || terms.original_anchor.tick.get() < initial_tick
        || terms.high_tick.get() > retained_tick
    {
        return Err(denied(
            "project stock watch changed its parent lineage or rejection horizon",
        ));
    }
    Ok(())
}

pub(super) fn from_reference(reference: &Value) -> Result<Option<Checkpoint>> {
    let Some(value) = reference.get("stock_watch") else {
        return Ok(None);
    };
    let checkpoint = Checkpoint::from_json(value)?;
    if reference["schema"] != "dfmcp.project-production-recovery/1" {
        return Err(invalid(
            "stock watch requires exact project production custody",
        ));
    }
    for family in ["workforce_journal_id", "order_journal_id"] {
        digest(
            reference["native_custody"][family]
                .as_str()
                .ok_or_else(|| invalid("stock watch lost its original native custodian"))?,
        )?;
    }
    let floor = reference["observation_tick_floor"]
        .as_u64()
        .ok_or_else(|| invalid("stock-watch reference lost its consumed tick floor"))?;
    if floor > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
        || checkpoint.terms()?.high_tick.get() > floor
    {
        return Err(denied(
            "stock-watch source history exceeds the retained rejection floor",
        ));
    }
    Ok(Some(checkpoint))
}

pub(crate) fn reference_high_tick(reference: &Value) -> Result<Option<u64>> {
    from_reference(reference)?
        .map(|checkpoint| checkpoint.terms().map(|terms| terms.high_tick.get()))
        .transpose()
}

/// Legacy references omit this field. After first publication no frame may
/// erase it, roll its source floors back, replace workers, or clear terminal
/// history, including after the first effect-bearing continuation.
pub(crate) fn verify_reference_continuity(before: &Value, after: &Value) -> Result<()> {
    match (from_reference(before)?, from_reference(after)?) {
        (None, None) => Ok(()),
        (Some(previous), Some(current)) => previous.successor(&current),
        (None, Some(checkpoint))
            if before.get("recovery") == Some(&Value::Null)
                && after.get("recovery") == Some(&Value::Null) =>
        {
            checkpoint.require_initial()
        }
        _ => Err(denied(
            "project child erased or introduced an unacknowledged stock watch",
        )),
    }
}

/// A production intent may carry the historical watch into a first positive
/// original. Both its prepublication checkpoint and later descendants must keep
/// that same request and monotone lifecycle; pending recovery retains newer
/// failed-call rejection evidence from the actual parent checkpoint.
pub(super) fn verify_intent_watch(intent: &Value, current: &Value) -> Result<()> {
    match (from_reference(intent)?, from_reference(current)?) {
        (None, None) => Ok(()),
        (Some(expected), Some(actual)) => {
            if current["recovery"].is_null() {
                // Before publication the acknowledged checkpoint may precede
                // the final source read that sized this exact intent.
                if expected.successor(&actual).is_err() {
                    actual.successor(&expected)?;
                }
                // Source history may be newer on either side of the intent;
                // the parent frame sequence independently enforces monotonicity.
                Ok(())
            } else {
                expected.successor(&actual)
            }
        }
        _ => Err(denied(
            "project intent lost its stock-watch predecessor custody",
        )),
    }
}

pub(super) fn join_intent_watch(reference: &mut Value, prior: &Value) -> Result<()> {
    match (from_reference(reference)?, from_reference(prior)?) {
        (None, None) => Ok(()),
        (Some(expected), Some(actual)) => {
            if expected.successor(&actual).is_ok() {
                reference["stock_watch"] = actual.to_json();
            } else {
                actual.successor(&expected)?;
            }
            Ok(())
        }
        _ => Err(denied(
            "project publication lost its original stock-watch history",
        )),
    }
}

impl Runtime {
    /// The parent calls this only after its terminal stop is independently
    /// acknowledged. Withdrawal is local and idempotent, including Offline:
    /// no native owner, positive original or predecessor history is changed.
    pub(crate) fn retire_project_stock_watch_after_stop(
        &mut self,
        goal: &ProjectStockGoal,
    ) -> Result<()> {
        self.project_workshop_origin(goal)?;
        if self.original.is_some() {
            return Ok(());
        }
        if let Some(checkpoint) = self.project_stock_watch_checkpoint() {
            checkpoint.validate_goal(goal)?;
        }
        if let Some(watch) = &mut self.stock_target_no_work {
            watch.cancelled = true;
            watch.monitor.invalidate_current_sample();
        }
        if let Some(watch) = &mut self.stock_targets_no_work {
            watch.cancelled = true;
            watch.monitor.invalidate_current_sample();
        }
        self.basis = None;
        self.planning_inspection.withdraw();
        self.parent_goal = None;
        Ok(())
    }

    pub(super) fn project_stock_watch_checkpoint(&self) -> Option<Checkpoint> {
        if self.config.project_binding.is_none() {
            return None;
        }
        self.stock_target_no_work
            .as_ref()
            .map(Checkpoint::scalar)
            .or_else(|| {
                self.stock_targets_no_work
                    .as_ref()
                    .map(Checkpoint::portfolio)
            })
            .or_else(|| self.retained_project_stock_watch.clone())
    }

    /// Invoked only after all imported donors, native custodians and any
    /// complete original archive have independently passed ordinary recovery.
    pub(super) fn restore_project_stock_watch(&mut self, work: &mut Work) -> Result<()> {
        let Some(checkpoint) = self.config.project_stock_watch.take() else {
            return Ok(());
        };
        if self.config.project_recovery.is_none() || self.config.project_binding.is_none() {
            return Err(denied(
                "stock-watch recovery requires acknowledged project custody",
            ));
        }
        let terms = checkpoint.terms()?;
        let bytes = decode(&checkpoint.checkpoint_hex)?;
        let context = work.take(MIB)?;
        let has_original = self.original.is_some();
        if let Some(original) = &self.original {
            if checkpoint.cancelled
                || prerequisite_binding::requirements(original)?.stock_watch
                    != Some(terms.goal_witness)
            {
                return Err(denied(
                    "production original differs from its retained stock-watch predecessor",
                ));
            }
        }
        match &checkpoint.request {
            OriginalRequest::Single(request) => {
                let monitor = StockTargetWatch::recover(&bytes, &context)?;
                if has_original {
                    if monitor.achieved_at().is_some() || monitor.fenced() {
                        return Err(denied(
                            "completed or fenced stock watch cannot publish a production original",
                        ));
                    }
                } else {
                    self.stock_target_no_work = Some(StockTargetNoWork {
                        assessment: monitor.original().clone(),
                        request: request.clone(),
                        monitor,
                        cancelled: checkpoint.cancelled,
                    });
                }
            }
            OriginalRequest::Portfolio(request) => {
                let monitor = StockTargetsWatch::recover(&bytes, &context)?;
                if has_original {
                    if monitor.achieved_at().is_some() || monitor.fenced() {
                        return Err(denied(
                            "completed or fenced portfolio watch cannot publish a production original",
                        ));
                    }
                } else {
                    self.stock_targets_no_work = Some(StockTargetsNoWork::recovered(
                        request.clone(),
                        monitor,
                        checkpoint.cancelled,
                    )?);
                }
            }
        }
        self.observation_tick_floor = self.observation_tick_floor.max(terms.high_tick);
        self.retained_project_stock_watch = Some(checkpoint);
        Ok(())
    }

    pub(super) fn next_stock_watch_cursor(
        &self,
        mut cursor: ObservationCursor,
    ) -> Result<ObservationCursor> {
        let retained = self
            .stock_target_no_work
            .as_ref()
            .map(|goal| goal.monitor.high_cursor())
            .or_else(|| {
                self.stock_targets_no_work
                    .as_ref()
                    .map(|goal| goal.monitor.high_cursor())
            });
        if let Some(retained) = retained {
            if cursor.epoch != retained.epoch {
                return Err(denied(
                    "stock-watch continuation changed its original observation epoch",
                ));
            }
            cursor.sequence = cursor.sequence.max(
                retained
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| exhausted("stock-watch continuation cursor exhausted"))?,
            );
        }
        Ok(cursor)
    }
}

#[cfg(test)]
#[path = "project_stock_watch_tests.rs"]
pub(crate) mod tests;
