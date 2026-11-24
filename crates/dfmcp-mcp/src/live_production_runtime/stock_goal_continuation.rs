//! Explicit first production for an originally satisfied, unfinished stock goal.
//! The caller selects only the retained witness. Fresh source reads determine
//! any finite deficit; the original workers, targets and timing cannot change.
use super::*;
use dfmcp_core::RequestId;
use dfmcp_world::PredicateTruth;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StockGoalContinuationRequest {
    kind: String,
    goal_witness: String,
}

impl StockGoalContinuationRequest {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 1024 {
            return Err(exhausted("stock-goal continuation exceeds 1024 bytes"));
        }
        let request: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed stock-goal continuation request"))?;
        request.witness()?;
        Ok(request)
    }

    fn witness(&self) -> Result<Digest32> {
        if self.kind != "continue_stock_goal" {
            return Err(invalid(
                "stock continuation requires kind=continue_stock_goal",
            ));
        }
        digest(&self.goal_witness)
    }
}

enum OriginalRequest {
    Single(StockTargetRequest),
    Portfolio(StockTargetsRequest),
}

/// This request-local value is minted only from a retained unfinished watch.
/// An archive marker is historical identity and cannot recreate this value.
#[derive(Clone, Copy)]
pub(super) struct StockGoalContinuation {
    witness: Digest32,
    portfolio: bool,
    session: SessionId,
    request: RequestId,
}

impl StockGoalContinuation {
    pub(super) const fn witness(self) -> Digest32 {
        self.witness
    }

    pub(super) fn verify(&self, owner: &Runtime, c: &OperationContext) -> Result<()> {
        if self.session != c.session_id || self.request != c.request_id {
            return Err(denied("stock continuation changed its compiling request"));
        }
        let (witness, portfolio, _, _, _) = owner.unfinished_stock_watch(c)?;
        if witness != self.witness || portfolio != self.portfolio {
            return Err(denied("stock continuation lost its exact original watch"));
        }
        Ok(())
    }

    pub(super) fn single_request(
        &self,
        owner: &Runtime,
        request: &StockTargetRequest,
        c: &OperationContext,
    ) -> Result<()> {
        self.verify(owner, c)?;
        let mut original = owner
            .stock_target_no_work
            .as_ref()
            .ok_or_else(|| denied("stock continuation lost its scalar original"))?
            .request
            .clone();
        original.expected_observation = request.expected_observation.clone();
        if self.portfolio || original != *request {
            return Err(denied(
                "stock continuation changed original targets, workers or timing",
            ));
        }
        Ok(())
    }

    pub(super) fn portfolio_request(
        &self,
        owner: &Runtime,
        request: &StockTargetsRequest,
        c: &OperationContext,
    ) -> Result<()> {
        self.verify(owner, c)?;
        let mut original = owner
            .stock_targets_no_work
            .as_ref()
            .ok_or_else(|| denied("stock continuation lost its portfolio original"))?
            .request
            .clone();
        original.expected_observation = request.expected_observation.clone();
        if !self.portfolio || original != *request {
            return Err(denied(
                "stock continuation changed original targets, workers or timing",
            ));
        }
        Ok(())
    }
}

impl Runtime {
    /// Advisory next action only. The explicit request reacquires all sources
    /// and current authority; a displayed False sample never sizes an order.
    pub(super) fn stock_continuation_request_json(&self) -> Value {
        if self.mode != Mode::Control
            || self.fenced
            || self.initialization_failed
            || self.original.is_some()
            || self.frontier.is_some()
            || !self.orders_enabled()
            || self.clock_withholds_goal()
            || self.any_portfolio_clock_pending()
        {
            return Value::Null;
        }
        let identity = match (&self.stock_target_no_work, &self.stock_targets_no_work) {
            (Some(goal), None)
                if !goal.cancelled
                    && !goal.monitor.fenced()
                    && goal.monitor.achieved_at().is_none()
                    && goal.monitor.latest().is_some_and(|sample| {
                        sample.anchor == self.anchor
                            && sample.current_truth == PredicateTruth::False
                    })
                    && self
                        .authority_tick_floor()
                        .max(goal.monitor.high_tick())
                        .get()
                        < goal.request.deadline_tick =>
            {
                Some(goal.monitor.witness())
            }
            (None, Some(goal))
                if !goal.cancelled
                    && !goal.monitor.fenced()
                    && goal.monitor.achieved_at().is_none()
                    && goal.monitor.latest().is_some_and(|sample| {
                        sample.anchor == self.anchor
                            && sample.current_truth == PredicateTruth::False
                    })
                    && self
                        .authority_tick_floor()
                        .max(goal.monitor.high_tick())
                        .get()
                        < goal.request.deadline_tick =>
            {
                Some(goal.monitor.witness())
            }
            _ => None,
        };
        identity.map_or(Value::Null, |witness| {
            json!({
                "kind":"continue_stock_goal", "goal_witness":witness.to_hex(),
            })
        })
    }

    pub(super) fn stock_continuation_json(&self) -> Value {
        self.original
            .as_ref()
            .and_then(|plan| prerequisite_binding::requirements(plan).ok())
            .and_then(|origins| origins.stock_watch)
            .map_or(Value::Null, |witness| {
                json!({
                    "schema":"dfmcp.stock-goal-continuation/1",
                    "original_watch_witness":witness.to_hex(),
                    "evidence_scope":"historical_predecessor_identity",
                    "recovery_restores_watch":false,
                    "recovery_restores_current_proof":false,
                    "automatic_maintenance":false,
                })
            })
    }

    /// The first original must start from a fresh empty native manager queue.
    /// Later preparation and dispatch retain their independent queue checks.
    pub(super) fn check_stock_continuation_queue(&mut self, work: &mut Work) -> Result<()> {
        let queue = self.config.orders.stock_target_queue(
            &work.take(orders::native::STOCK_TARGET_QUEUE_BYTES)?,
            &mut self.observation_tick_floor,
        )?;
        let current = work.current()?;
        let pins = self.config.orders.acceptance();
        if queue.fortress_id() != current.anchor.fortress_id
            || queue.world_folder() != pins.world_folder
            || queue.site_id() != pins.site_id
            || queue.generation() != pins.work_order_generation
            || !queue.paused()
            || queue.tick() != current.anchor.tick.get()
            || !queue.order_ids().is_empty()
        {
            return Err(denied(
                "stock continuation requires the fresh empty paused native order queue",
            ));
        }
        Ok(())
    }

    fn unfinished_stock_watch(
        &self,
        c: &OperationContext,
    ) -> Result<(Digest32, bool, u64, u64, u32)> {
        self.check(c, true)?;
        if self.original.is_some()
            || self.frontier.is_some()
            || self.mode != Mode::Control
            || self.initialization_failed
            || self.clock_withholds_goal()
            || self.any_portfolio_clock_pending()
            || self.clock.as_ref().is_some_and(ProductionClock::pending)
            || self
                .inventory_clock
                .as_ref()
                .is_some_and(ProductionInventoryClock::pending)
        {
            return Err(denied(
                "stock continuation requires an unfinished no-work owner with settled clocks",
            ));
        }
        let (
            witness,
            portfolio,
            cancelled,
            fenced,
            achieved,
            high_tick,
            deadline,
            cadence,
            samples,
        ) = match (&self.stock_target_no_work, &self.stock_targets_no_work) {
            (Some(goal), None) => (
                goal.monitor.witness(),
                false,
                goal.cancelled,
                goal.monitor.fenced(),
                goal.monitor.achieved_at(),
                goal.monitor.high_tick(),
                goal.request.deadline_tick,
                goal.request.poll_interval_ticks,
                goal.request.stable_samples,
            ),
            (None, Some(goal)) => (
                goal.monitor.witness(),
                true,
                goal.cancelled,
                goal.monitor.fenced(),
                goal.monitor.achieved_at(),
                goal.monitor.high_tick(),
                goal.request.deadline_tick,
                goal.request.poll_interval_ticks,
                goal.request.stable_samples,
            ),
            _ => {
                return Err(denied(
                    "stock continuation requires exactly one retained original watch",
                ));
            }
        };
        if witness == Digest32::ZERO
            || cancelled
            || fenced
            || achieved.is_some()
            || c.anchor
                .tick
                .max(high_tick)
                .max(self.authority_tick_floor())
                .get()
                >= deadline
        {
            return Err(denied(
                "cancelled, completed, fenced or expired stock goals cannot continue",
            ));
        }
        Ok((witness, portfolio, deadline, cadence, samples))
    }

    pub(crate) fn continue_stock_goal(
        &mut self,
        request: StockGoalContinuationRequest,
        c: &OperationContext,
    ) -> Result<View> {
        self.continue_stock_goal_recorded(request, c, None)
    }

    pub(super) fn continue_stock_goal_recorded(
        &mut self,
        request: StockGoalContinuationRequest,
        c: &OperationContext,
        publication: Option<project::ProjectPublication<'_>>,
    ) -> Result<View> {
        let witness = request.witness()?;
        let (retained, portfolio, _, _, _) = self.unfinished_stock_watch(c)?;
        if witness != retained {
            return Err(denied("stock continuation names another original goal"));
        }
        let (original, units) = if let Some(goal) = &self.stock_target_no_work {
            (
                OriginalRequest::Single(goal.request.clone()),
                goal.request.units.clone(),
            )
        } else if let Some(goal) = &self.stock_targets_no_work {
            (
                OriginalRequest::Portfolio(goal.request.clone()),
                goal.request.units.clone(),
            )
        } else {
            return Err(denied("stock continuation lost its retained request"));
        };
        let mut authority = c.clone();
        authority.anchor.tick = authority.anchor.tick.max(self.authority_tick_floor());
        authority.authorize(
            Capability::ConfigureProduction,
            RiskTier::Guarded,
            &[],
            None,
        )?;
        match &original {
            OriginalRequest::Single(request) if request.assign_labor => authority.authorize(
                Capability::ConfigureLabor,
                RiskTier::Guarded,
                &request.unit_ids()?,
                None,
            )?,
            OriginalRequest::Portfolio(request) if request.assign_labor => authority.authorize(
                Capability::ConfigureLabor,
                RiskTier::Guarded,
                &request.unit_ids()?,
                None,
            )?,
            _ => {}
        }
        let continuation = StockGoalContinuation {
            witness,
            portfolio,
            session: c.session_id,
            request: c.request_id,
        };
        // Preserve the old monitor through every prepublication refusal. The
        // shared read acquires the exact original citizen selection anew; a
        // previous False sample or planning census is never a current baseline.
        let selection = ObserveRequest::parse(&json!({"units":units}).to_string())?;
        let mut work = Work::new(c)?;
        let result = (|| {
            self.basis = None;
            self.planning_inspection.withdraw();
            let basis = self.acquire_planning_basis(&selection, &mut work)?;
            self.anchor = basis.projection.snapshot.anchor();
            let observation = basis.seal.to_hex();
            self.basis = Some(basis);
            work.context.anchor = self.anchor;
            continuation.verify(self, &work.current()?)?;
            match original {
                OriginalRequest::Single(mut original) => {
                    original.expected_observation = observation;
                    self.plan_stock_target_fresh(
                        original,
                        &work.current()?,
                        publication,
                        Some(continuation),
                    )
                }
                OriginalRequest::Portfolio(mut original) => {
                    original.expected_observation = observation;
                    self.plan_stock_targets_fresh(
                        original,
                        &work.current()?,
                        publication,
                        Some(continuation),
                    )
                }
            }
        })();
        if result.is_err() {
            // A failed new read/publication withdraws current proof and breaks
            // unfinished stability, but never rolls decoded monotone floors
            // or the original watch identity/request/deadline backward.
            self.abandon_preparation();
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_selects_only_one_exact_original_witness() -> Result<()> {
        let witness = Digest32::of_bytes(b"retained original stock goal").to_hex();
        let raw = json!({"kind":"continue_stock_goal","goal_witness":witness}).to_string();
        assert_eq!(
            StockGoalContinuationRequest::parse(&raw)?
                .witness()?
                .to_hex(),
            witness
        );
        for field in [
            "targets",
            "units",
            "deadline_tick",
            "assign_labor",
            "amount",
            "expected_observation",
        ] {
            let mut value: Value =
                serde_json::from_str(&raw).map_err(|_| invalid("test request malformed"))?;
            value[field] = json!(1);
            assert!(StockGoalContinuationRequest::parse(&value.to_string()).is_err());
        }
        for bad in ["0".repeat(64), witness.to_uppercase(), "short".into()] {
            assert!(
                StockGoalContinuationRequest::parse(
                    &json!({"kind":"continue_stock_goal","goal_witness":bad}).to_string()
                )
                .is_err()
            );
        }
        assert!(StockGoalContinuationRequest::parse(&format!(
            "{{\"kind\":\"continue_stock_goal\",\"goal_witness\":\"{witness}\",\"goal_witness\":\"{witness}\"}}"
        )).is_err());
        Ok(())
    }
}
