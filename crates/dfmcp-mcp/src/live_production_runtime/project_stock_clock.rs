//! A resource parent's separate clock may advance only its unchanged no-order
//! stock watch. These typed checks cannot turn a native receipt into stock proof.
use super::*;
use dfmcp_world::PredicateTruth;

#[derive(Clone, Copy, Debug)]
pub(crate) struct StockWatchClockBasis {
    pub(crate) anchor: StateAnchor,
    pub(crate) goal_witness: Digest32,
    pub(crate) next_sample_tick: u64,
}

impl Runtime {
    pub(crate) fn project_stock_clock_shell(
        &self,
        goal: &ProjectStockGoal,
    ) -> Result<orders::native::NativeShell> {
        self.project_status(goal)?;
        self.config.current()?;
        Ok(self.config.orders.clone())
    }

    pub(crate) fn project_has_stock_watch(&self, goal: &ProjectStockGoal) -> Result<bool> {
        self.project_status(goal)?;
        Ok(self.original.is_none()
            && self.has_active_stock_watch()
            && !self.fenced
            && !self.initialization_failed)
    }

    /// Only a currently qualified True sample can justify spending time on
    /// stability. Missing stock instead follows the original continuation.
    pub(crate) fn project_stock_clock_basis(
        &self,
        goal: &ProjectStockGoal,
    ) -> Result<StockWatchClockBasis> {
        let status = self.project_status(goal)?;
        if self.original.is_some()
            || !status.clocks_settled
            || self.clock_withholds_goal()
            || self.fenced
            || self.initialization_failed
            || self.anchor.tick < self.authority_tick_floor()
            || self.anchor.tick.get() >= goal.deadline_tick()
        {
            return Err(denied(
                "stock-watch time requires the original unfinished no-order goal and settled sources",
            ));
        }
        let (anchor, witness, truth, samples, counted_from, expired, fenced, cancelled, achieved) =
            if let Some(watch) = &self.stock_target_no_work {
                goal.validate_request(&watch.request)?;
                let sample = watch.monitor.latest().ok_or_else(|| {
                    denied("sample the original stock watch before planning time")
                })?;
                (
                    sample.anchor,
                    watch.monitor.witness(),
                    sample.current_truth,
                    sample.stable_samples_observed,
                    sample
                        .last_counted_tick
                        .unwrap_or(watch.assessment.original_anchor().tick),
                    sample.expired,
                    watch.monitor.fenced(),
                    watch.cancelled,
                    sample.achieved_at.is_some(),
                )
            } else if let Some(watch) = &self.stock_targets_no_work {
                goal.validate_targets_request(&watch.request)?;
                let sample = watch.monitor.latest().ok_or_else(|| {
                    denied("sample every original stock target before planning time")
                })?;
                (
                    sample.anchor,
                    watch.monitor.witness(),
                    sample.current_truth,
                    sample.stable_samples_observed,
                    sample.last_counted_tick.unwrap_or(
                        watch
                            .monitor
                            .originals()
                            .first()
                            .ok_or_else(|| denied("the stock portfolio lost its original cohort"))?
                            .original_anchor()
                            .tick,
                    ),
                    sample.expired,
                    watch.monitor.fenced(),
                    watch.cancelled,
                    sample.achieved_at.is_some(),
                )
            } else {
                return Err(denied("the project has no original no-order stock watch"));
            };
        if anchor != self.anchor
            || truth != PredicateTruth::True
            || samples >= goal.stable_samples()
            || expired
            || fenced
            || cancelled
            || achieved
        {
            return Err(denied(
                "stock-watch time needs current complete stock truth with stability still outstanding",
            ));
        }
        let next_sample_tick = counted_from
            .get()
            .checked_add(goal.poll_interval_ticks())
            .filter(|tick| *tick > anchor.tick.get() && *tick <= goal.deadline_tick())
            .ok_or_else(|| {
                denied("the original stock cadence has no remaining bounded interval")
            })?;
        Ok(StockWatchClockBasis {
            anchor,
            goal_witness: witness,
            next_sample_tick,
        })
    }

    /// This is a full qualified citizen/operations census plus actual child
    /// custody verification, including the original Workshop/Manager donors.
    /// The caller prepays its complete allowance and checks the returned tick
    /// against the independently observed Run/1.13 source at the final edge.
    pub(crate) fn project_sample_stock_clock(
        &mut self,
        goal: &ProjectStockGoal,
        c: &OperationContext,
    ) -> Result<StockWatchClockBasis> {
        self.check(c, true)?;
        if !self.project_has_stock_watch(goal)? {
            return Err(denied(
                "stock-watch time cannot create or replace an original production plan",
            ));
        }
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        if !self.opening_inventory_clock || !self.config.orders.inventory_clock_enabled() {
            return Err(denied(
                "stock-watch time requires independent inventory-clock authority",
            ));
        }
        self.wait_stock_target(c)?;
        self.project_stock_clock_basis(goal)
    }

    /// An intentional bounded interval hides the old sample while retaining
    /// its authentic counted history. Unknown/failed observations continue to
    /// use abandon_preparation(), which interrupts and resets that history.
    pub(crate) fn retain_project_stock_clock_tick(&mut self, tick: u64) -> Result<()> {
        if self.config.project_binding.is_none()
            || tick > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
        {
            return Err(denied(
                "invalid original project stock-clock authority floor",
            ));
        }
        self.retain_parent_authority_floor(tick);
        if let Some(watch) = &mut self.stock_target_no_work {
            watch.monitor.withhold_current_sample();
        }
        if let Some(watch) = &mut self.stock_targets_no_work {
            watch.monitor.withhold_current_sample();
        }
        self.basis = None;
        self.parent_goal = None;
        Ok(())
    }

    pub(crate) fn project_stock_clock_view(&self) -> Value {
        let mut value = self.opening_recovery_reference();
        value["ok"] = json!(true);
        value["current_state_proven"] = json!(false);
        value["original_goal_proven"] = json!(false);
        value
    }
}
