//! A bounded queued-service allowance for new laboratory quota work.
//!
//! These are current physical orders, independent of process-local action
//! handles. Their remaining service is a conservative delay input, not future
//! stock or proof that either old or new work will complete. No source changes,
//! assignment, cancellation, clock movement or effect dispatch happens here.

use super::*;
use dfmcp_core::{GameTick, StateAnchor, StepId};
use dfmcp_intent::{Intent, ObligationSpec, derive_step_idempotency_key};

const MAX_WORKLOAD_ORDERS: usize = 128;
const MAX_WORKLOAD_ROWS: usize = 16;

struct ExistingOrder {
    id: EntityId,
    job: &'static str,
    remaining: u64,
    partial: u64,
}

pub(super) struct ProductionWorkload {
    anchor: StateAnchor,
    orders: Vec<ExistingOrder>,
    /// Sum of outstanding registered worker service, before the conservative
    /// allowance used to absorb service competition and settlement boundaries.
    service_ticks: u64,
    allowance_ticks: u64,
}

fn unavailable(record: &EntityRecord, field: &str) -> DfmcpError {
    DfmcpError::new(
        ErrorCode::PreconditionsFailed,
        format!(
            "production workload requires source-qualified {field} for existing work order {}; inspect or reconcile that original work before adding quota work",
            record.id.get(),
        ),
    )
}

fn value<'a>(record: &'a EntityRecord, field: &str, tick: GameTick) -> Result<&'a Value> {
    record
        .fields
        .get(field)
        .and_then(|fact| laboratory_fact_value(fact, tick))
        .ok_or_else(|| unavailable(record, field))
}

fn unsigned(record: &EntityRecord, field: &str, tick: GameTick) -> Result<u64> {
    match value(record, field, tick)? {
        Value::U64(number) => Ok(*number),
        _ => Err(unavailable(record, field)),
    }
}

fn horizon_error() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "existing production and new quota work exceed the bounded one-year planning horizon; observe or drain existing work before adding more",
    )
}

impl ProductionWorkload {
    pub(super) fn capture(snapshot: &WorldSnapshot, jobs: &[(&str, &str)]) -> Result<Self> {
        if snapshot.graph.entities.len() > MAX_PRODUCTION_SETUP_ENTITIES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "production workload exceeds the 65,536-entity inspection bound",
            ));
        }
        let mut result = Self {
            anchor: snapshot.anchor(),
            orders: Vec::new(),
            service_ticks: 0,
            allowance_ticks: 0,
        };
        for record in snapshot.graph.entities.values() {
            if record.kind != EntityKind::WorkOrder {
                continue;
            }
            match value(record, effects::STATUS_FIELD, snapshot.tick)? {
                Value::Text(status)
                    if status == effects::STATUS_COMPLETE
                        || status == effects::STATUS_CANCELLED =>
                {
                    continue;
                }
                Value::Text(status) if status == effects::STATUS_ACTIVE => {}
                _ => return Err(unavailable(record, effects::STATUS_FIELD)),
            }
            let (job, output) = match value(record, "job_token", snapshot.tick)? {
                Value::Text(job) if job == "BREW_DRINK" => ("BREW_DRINK", "DRINK"),
                Value::Text(job) if job == "PREPARE_MEAL" => ("PREPARE_MEAL", "FOOD"),
                Value::Text(job) if job == "COOK_MEAL" => ("COOK_MEAL", "FOOD"),
                // No shared worker/workshop capacity is registered for other
                // job tokens, so their unmodeled resource use is not invented.
                Value::Text(_) => continue,
                _ => return Err(unavailable(record, "job_token")),
            };
            let remaining = unsigned(record, effects::AMOUNT_REMAINING_FIELD, snapshot.tick)?;
            let partial = unsigned(record, "work_ticks", snapshot.tick)?;
            if remaining == 0 || partial >= effects::WORK_ORDER_TICKS_PER_UNIT {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!(
                        "existing active production order {} has noncanonical remaining/partial work; reconcile its original state before planning",
                        record.id.get(),
                    ),
                ));
            }
            if jobs
                .iter()
                .any(|(requested_output, _)| *requested_output == output)
            {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!(
                        "production quota overlaps existing physical work: observed order {} has status=active, job_token={job}, amount_remaining={remaining}, work_ticks={partial} at tick {} and source {}; a second {output} quota-gated order cannot establish independent progress while that producer remains active. Wait for its observed completion, reconcile it, or explicitly cancel its original work before replanning. Its possible future output has not been counted as current stock",
                        record.id.get(),
                        snapshot.tick.0,
                        snapshot.state_hash.to_hex(),
                    ),
                ));
            }
            if result.orders.len() >= MAX_WORKLOAD_ORDERS {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "production workload exceeds the 128 active registered-order bound",
                ));
            }
            let service = remaining
                .checked_mul(effects::WORK_ORDER_TICKS_PER_UNIT)
                .and_then(|ticks| ticks.checked_sub(partial))
                .ok_or_else(horizon_error)?;
            result.service_ticks = result
                .service_ticks
                .checked_add(service)
                .filter(|ticks| *ticks <= effects::MAX_DEFAULT_OBLIGATION_TICKS / 2)
                .ok_or_else(horizon_error)?;
            result.orders.push(ExistingOrder {
                id: record.id,
                job,
                remaining,
                partial,
            });
        }
        // Count every prior registered unit as queued even where some work
        // could run in parallel or is presently conditional-blocked. This
        // never relies on a guessed canonical priority for the new order.
        result.allowance_ticks = result
            .service_ticks
            .checked_mul(2)
            .ok_or_else(horizon_error)?;
        Ok(result)
    }

    pub(super) fn analysis(&self) -> Option<Json> {
        if self.orders.is_empty() {
            return None;
        }
        Some(json!({
            "epistemic_state": "predicted",
            "method": "conservative_registered_production_service_allowance",
            "source_state_hash": self.anchor.state_hash.to_hex(),
            "source_tick": self.anchor.tick.0,
            "observed_existing_order_count": self.orders.len(),
            "existing_orders": self.orders.iter().take(MAX_WORKLOAD_ROWS).map(|order| json!({
                "entity_id": order.id.get().to_string(),
                "job_token": order.job,
                "status": "active",
                "amount_remaining": order.remaining,
                "earned_work_ticks": order.partial,
                "epistemic_state": "observed",
            })).collect::<Vec<_>>(),
            "existing_orders_omitted": self.orders.len().saturating_sub(MAX_WORKLOAD_ROWS),
            "remaining_registered_service_ticks": self.service_ticks,
            "queued_service_allowance_ticks": self.allowance_ticks,
            "allowance_factor": 2,
            "completion_guaranteed": false,
            "future_output_counted_as_stock": false,
            "assumptions": "all prior registered production is counted conservatively as queued service, including currently blocked work; actual completion still requires current eligible workers/workshops, conditions, authorized foreground observations and no interfering new work",
        }))
    }

    pub(super) fn apply_horizon(&self, actions: &str, intent: &mut Intent) -> Result<()> {
        if self.allowance_ticks == 0 {
            // Preserve the exact established uncontended plan, including
            // omitted default obligation fields before ordinary preparation.
            return Ok(());
        }
        if intent.anchor != self.anchor || intent.requested_actions != parse_steps(actions)? {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "production workload allowance belongs to a different source or action program",
            ));
        }
        let obligations = self.obligations(&intent.requested_actions, false, |index, action| {
            let step = StepId::new(u32::try_from(index).map_err(|_| horizon_error())?);
            Ok(derive_step_idempotency_key(
                intent.id,
                intent.anchor,
                step,
                action,
            ))
        })?;
        for (requested, obligation) in intent.requested_actions.iter_mut().zip(obligations) {
            requested.obligation = obligation;
        }
        Ok(())
    }

    /// Exact relative horizon of the default obligations that the new compiler
    /// will seal. A preview key changes predicates only, never their deadlines.
    pub(super) fn horizon_ticks(&self, actions: &str) -> Result<u64> {
        let obligations = self.obligations(&parse_steps(actions)?, true, |index, _| {
            Ok(format!("production-consumption-horizon-{index}"))
        })?;
        Ok(self.latest_deadline(&obligations).0 - self.anchor.tick.0)
    }

    /// Bind the same complete horizon used to size consumption reserves to real
    /// step identities. Unlike the legacy path this also seals zero-backlog
    /// defaults, so preview and preparation cannot choose different horizons.
    pub(super) fn apply_complete_horizon(
        &self,
        actions: &str,
        intent: &mut Intent,
        expected_ticks: u64,
    ) -> Result<()> {
        if intent.anchor != self.anchor || intent.requested_actions != parse_steps(actions)? {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "production consumption horizon belongs to a different source or action program",
            ));
        }
        let obligations = self.obligations(&intent.requested_actions, true, |index, action| {
            let step = StepId::new(u32::try_from(index).map_err(|_| horizon_error())?);
            Ok(derive_step_idempotency_key(
                intent.id,
                intent.anchor,
                step,
                action,
            ))
        })?;
        if self.latest_deadline(&obligations).0 - self.anchor.tick.0 != expected_ticks {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "production consumption reserve and sealed obligation horizons disagree",
            ));
        }
        // A refusal above leaves the caller's original intent unchanged.
        for (requested, obligation) in intent.requested_actions.iter_mut().zip(obligations) {
            requested.obligation = obligation;
        }
        Ok(())
    }

    fn latest_deadline(&self, obligations: &[Option<ObligationSpec>]) -> GameTick {
        obligations
            .iter()
            .filter_map(|obligation| obligation.as_ref().map(|value| value.deadline_tick))
            .max()
            .unwrap_or(self.anchor.tick)
    }

    fn obligations(
        &self,
        actions: &[RequestedAction],
        strict_service_bound: bool,
        mut key: impl FnMut(usize, &Action) -> Result<String>,
    ) -> Result<Vec<Option<ObligationSpec>>> {
        let horizon_limit = self
            .anchor
            .tick
            .checked_add(effects::MAX_DEFAULT_OBLIGATION_TICKS)
            .ok_or_else(horizon_error)?;
        let mut obligations: Vec<Option<ObligationSpec>> = Vec::with_capacity(actions.len());
        for (index, requested) in actions.iter().enumerate() {
            let mut start = self.anchor.tick;
            for dependency in &requested.depends_on {
                let previous = obligations.get(*dependency as usize).ok_or_else(|| {
                    invalid("production workload dependency must precede its consumer")
                })?;
                if let Some(obligation) = previous {
                    start = start.max(obligation.deadline_tick);
                }
            }
            let action = requested.action.normalized();
            if strict_service_bound && let Action::CreateWorkOrder { amount, .. } = &action {
                // The historical default caps an oversized obligation. The
                // consumption solver must refuse instead of mistaking that
                // cap for enough service to produce the requested quantity.
                u64::from(*amount)
                    .checked_mul(effects::WORK_ORDER_TICKS_PER_UNIT)
                    .and_then(|ticks| ticks.checked_mul(2))
                    .and_then(|ticks| ticks.checked_add(100))
                    .filter(|ticks| *ticks <= effects::MAX_DEFAULT_OBLIGATION_TICKS)
                    .ok_or_else(horizon_error)?;
            }
            if matches!(&action, Action::CreateWorkOrder { .. }) {
                // A condition-blocked existing order can become ready after
                // setup, so setup time cannot be assumed to drain its service.
                // Preserve the whole allowance after the prerequisite horizon.
                start = start
                    .checked_add(self.allowance_ticks)
                    .ok_or_else(horizon_error)?;
            }
            let key = key(index, &action)?;
            let obligation =
                effects::default_obligation(&action, &key, self.anchor.fortress_id, start)?;
            if obligation
                .as_ref()
                .is_some_and(|obligation| obligation.deadline_tick > horizon_limit)
            {
                return Err(horizon_error());
            }
            obligations.push(obligation);
        }
        Ok(obligations)
    }
}
