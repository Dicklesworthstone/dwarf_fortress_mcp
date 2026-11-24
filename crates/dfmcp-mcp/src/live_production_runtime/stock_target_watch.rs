//! Foreground observation of originally satisfied stock targets. No order,
//! labor assignment, native preparation, clock run or effect journal is needed.
//! The original sizing census is never reused as a temporal sample.
use super::*;
use dfmcp_adapter::semantic_work_orders::{
    INVENTORY_BASELINE_PROJECTION_BYTES, INVENTORY_BASELINE_QUALIFY_BYTES,
    InventoryBaselineProjection, OperationsGoalSource, STOCK_TARGETS_WATCH_BYTES,
};

const SAMPLE_BYTES: u64 = 16 * MIB + 2 * MIB + stock_target::CENSUS_BYTES;
const PORTFOLIO_SAMPLE_BYTES: u64 = 16 * MIB + 2 * MIB + stock_targets::CENSUS_BYTES;

impl Runtime {
    pub(super) fn no_work_goal_proven(&self) -> bool {
        !self.fenced
            && !self.initialization_failed
            && !self.clock_withholds_goal()
            && (self.stock_target_no_work.as_ref().is_some_and(|goal| {
                !goal.cancelled
                    && goal.monitor.current_goal_proven()
                    && goal
                        .monitor
                        .latest()
                        .is_some_and(|sample| sample.anchor == self.anchor)
            }) || self.stock_targets_no_work.as_ref().is_some_and(|goal| {
                !goal.cancelled
                    && goal.monitor.current_goal_proven()
                    && goal
                        .monitor
                        .latest()
                        .is_some_and(|sample| sample.anchor == self.anchor)
            }))
    }

    pub(super) fn no_work_goal_json(&self) -> Value {
        let Some(goal) = &self.stock_target_no_work else {
            return self.no_work_portfolio_json();
        };
        let current = goal.monitor.latest().filter(|sample| {
            !goal.cancelled && !self.clock_withholds_goal() && sample.anchor == self.anchor
        });
        json!({
            "schema": "dfmcp.stock-target-watch/1",
            "scope": "original_no_work_inventory_goal",
            "goal_witness": goal.monitor.witness().to_hex(),
            "assessment_witness": goal.assessment.witness().to_hex(),
            "recipe": goal.request.recipe,
            "item_type": goal.assessment.item_type(),
            "target_stack_units": goal.request.minimum_stack_units,
            "deadline_tick": goal.request.deadline_tick,
            "poll_interval_ticks": goal.request.poll_interval_ticks,
            "stable_samples_required": goal.request.stable_samples,
            "cancelled": goal.cancelled,
            "source_fenced": goal.monitor.fenced(),
            "consumed_tick_floor": goal.monitor.high_tick().get(),
            "continuation_request": self.stock_continuation_request_json(),
            "original_goal_proven": self.no_work_goal_proven(),
            "current_inventory_proven": self.no_work_goal_proven(),
            "sample": current.map(|sample| json!({
                "anchor": anchor_json(sample.anchor),
                "source_digest": sample.source_digest.to_hex(),
                "source_generation": sample.source_generation,
                "source_counters": sample.source_counters,
                "observed_stack_units": sample.observed_stack_units,
                "current_truth": goals::truth(sample.current_truth),
                "stable_samples_observed": sample.stable_samples_observed,
                "achieved_at_tick": sample.achieved_at.map(GameTick::get),
                "last_counted_tick": sample.last_counted_tick.map(GameTick::get),
                "deadline_reached": sample.deadline_reached,
                "expired": sample.expired,
                "continuous": sample.continuous,
                "witness": sample.witness.to_hex(),
            })),
            "fresh_for_current_call": false,
            "native_effect_dispatched": false,
            "worker_prerequisite_checked": false,
            "labor_assignment_planned": false,
            "continuous_maintenance": false,
            "production_attribution": "unknown",
            "inventory_semantics": "all_observed_type_stack_units",
            "sample_history_durable": false,
            "recovery_restores_current_proof": false,
        })
    }

    pub(super) fn no_work_recovery_json(&self) -> Value {
        let mut retained = self.no_work_goal_json();
        if retained.is_object() {
            retained["sample"] = Value::Null;
            retained["original_goal_proven"] = json!(false);
            retained["current_inventory_proven"] = json!(false);
            retained["continuation_request"] = Value::Null;
        }
        retained
    }

    pub(super) fn has_active_stock_watch(&self) -> bool {
        self.stock_target_no_work
            .as_ref()
            .is_some_and(|goal| !goal.cancelled)
            || self
                .stock_targets_no_work
                .as_ref()
                .is_some_and(|goal| !goal.cancelled)
    }

    pub(super) fn require_no_active_stock_watch(&self) -> Result<()> {
        if self.has_active_stock_watch() {
            return Err(denied(
                "retain the original no-work stock goal or explicitly cancel it before new planning",
            ));
        }
        Ok(())
    }

    pub(super) fn wait_stock_target(&mut self, c: &OperationContext) -> Result<View> {
        let result = self.wait_stock_target_inner(c);
        if let Some(goal) = &self.stock_target_no_work {
            self.observation_tick_floor = self.observation_tick_floor.max(goal.monitor.high_tick());
        }
        if let Some(goal) = &self.stock_targets_no_work {
            self.observation_tick_floor = self.observation_tick_floor.max(goal.monitor.high_tick());
        }
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }

    fn wait_stock_target_inner(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        if self.mode == Mode::Offline {
            return self.inspect(c);
        }
        let (cancelled, fenced, high_cursor, sample_bytes) =
            if let Some(goal) = &self.stock_target_no_work {
                (
                    goal.cancelled,
                    goal.monitor.fenced(),
                    goal.monitor.high_cursor(),
                    SAMPLE_BYTES,
                )
            } else if let Some(goal) = &self.stock_targets_no_work {
                (
                    goal.cancelled,
                    goal.monitor.fenced(),
                    goal.monitor.high_cursor(),
                    PORTFOLIO_SAMPLE_BYTES,
                )
            } else {
                return Err(invalid("no original no-work stock goal"));
            };
        if cancelled {
            return self.inspect(c);
        }
        if fenced || self.clock_withholds_goal() {
            return Err(denied(
                "no-work stock sampling requires continuous sources and settled clocks",
            ));
        }
        let cursor = ObservationCursor {
            epoch: self.anchor.cursor.epoch,
            sequence: self
                .anchor
                .cursor
                .sequence
                .max(high_cursor.sequence)
                .checked_add(1)
                .ok_or_else(|| exhausted("no-work stock cursor exhausted"))?,
        };
        let mut work = Work::new(c)?;
        // Reserve output and the complete pair of real reads before accessing
        // credentials or contacting either source. There is no hidden retry.
        let output = work.take(256 * MIB)?;
        let mut sample_work = Work::new(&work.take(sample_bytes)?)?;
        inventory_baseline::authorize_observe(
            &sample_work.current()?,
            self.authority_tick_floor(),
        )?;
        self.config.current()?;
        let capsule = self
            .config
            .workforce
            .acquire(&sample_work.take(16 * MIB)?)?;
        let citizens = project_live_capsule(&capsule, self.anchor.fortress_id, cursor)?;
        let anchor = citizens.snapshot.anchor();
        let previous_floor = self.observation_tick_floor;
        self.observation_tick_floor = previous_floor.max(anchor.tick);
        if anchor.tick < previous_floor.max(self.anchor.tick) {
            return Err(denied("no-work citizen observation regressed"));
        }
        sample_work.context.anchor = anchor;
        inventory_baseline::authorize_observe(
            &sample_work.current()?,
            self.observation_tick_floor,
        )?;
        self.config.workforce.acceptance().citizens(
            &capsule,
            &citizens,
            &sample_work.take(MIB)?,
        )?;
        self.config
            .orders
            .acceptance()
            .citizens(&capsule, &citizens, &sample_work.take(MIB)?)?;
        let mut source = self
            .config
            .orders
            .operations(&sample_work.take(16 * MIB)?)?;
        let operations = source.read_operations(&sample_work.take(16 * MIB)?)?;
        drop(source);
        self.observation_tick_floor = self.observation_tick_floor.max(operations.jobs.tick());
        if let Some(goal) = &mut self.stock_target_no_work {
            goal.monitor.note_capture(&operations)?;
        } else if let Some(goal) = &mut self.stock_targets_no_work {
            goal.monitor.note_capture(&operations)?;
        } else {
            return Err(invalid("no-work goal disappeared during its sample"));
        }
        inventory_baseline::authorize_observe(
            &sample_work.current()?,
            self.observation_tick_floor,
        )?;
        let projection = InventoryBaselineProjection::new(
            anchor,
            operations,
            &sample_work.take(INVENTORY_BASELINE_PROJECTION_BYTES)?,
        )?;
        let policy = self
            .config
            .orders
            .acceptance()
            .baseline(&projection, &sample_work.take(MIB)?)?;
        if let Some(goal) = &mut self.stock_target_no_work {
            goal.monitor.sample(
                &projection,
                policy,
                &sample_work.take(INVENTORY_BASELINE_QUALIFY_BYTES)?,
            )?;
        } else if let Some(goal) = &mut self.stock_targets_no_work {
            goal.monitor.sample(
                &projection,
                policy,
                &sample_work.take(STOCK_TARGETS_WATCH_BYTES)?,
            )?;
        } else {
            return Err(invalid("no-work goal disappeared during its sample"));
        }
        self.config.current()?;
        sample_work.current()?;
        self.anchor = anchor;
        // This fresh roster is observation, not another workforce choice or a
        // replacement original. A later new plan must explicitly observe again.
        self.basis = None;
        let mut output = output;
        output.anchor = anchor;
        output.budget.max_wall_millis = output
            .budget
            .max_wall_millis
            .min(work.current()?.budget.max_wall_millis);
        self.view(&mut Work::new(&output)?)
    }

    pub(super) fn cancel_stock_target(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, true)?;
        if let Some(goal) = &mut self.stock_target_no_work {
            goal.cancelled = true;
            goal.monitor.invalidate_current_sample();
        } else if let Some(goal) = &mut self.stock_targets_no_work {
            goal.cancelled = true;
            goal.monitor.invalidate_current_sample();
        } else {
            return Err(invalid("no original no-work stock goal"));
        }
        self.view(&mut Work::new(c)?)
    }
}
