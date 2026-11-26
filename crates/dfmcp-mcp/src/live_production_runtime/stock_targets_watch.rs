//! One foreground no-work stock portfolio: every minimum shares one census,
//! one immutable horizon and one joint stability interval.
use super::stock_targets_binding::StockTargetsBinding;
use super::*;
use dfmcp_adapter::semantic_work_orders::{StockTargetAssessment, StockTargetsWatch};

pub(super) struct StockTargetsNoWork {
    binding: StockTargetsBinding,
    assessment_witness: Digest32,
    pub(super) request: StockTargetsRequest,
    pub(super) monitor: StockTargetsWatch,
    pub(super) cancelled: bool,
}

impl StockTargetsNoWork {
    pub(super) fn recovered(
        request: StockTargetsRequest,
        monitor: StockTargetsWatch,
        cancelled: bool,
    ) -> Result<Self> {
        request.validate()?;
        let binding = if request.assign_labor {
            StockTargetsBinding::from_assessments_with_assignment(monitor.originals())?
        } else {
            StockTargetsBinding::from_assessments(monitor.originals())?
        };
        let assessment_witness = binding.witness()?;
        Ok(Self {
            binding,
            assessment_witness,
            request,
            monitor,
            cancelled,
        })
    }

    pub(super) fn new(
        assessments: Vec<StockTargetAssessment>,
        binding: StockTargetsBinding,
        request: StockTargetsRequest,
        context: &OperationContext,
    ) -> Result<Self> {
        request.validate()?;
        if assessments.len() != request.targets.len()
            || assessments
                .iter()
                .zip(&request.targets)
                .any(|(assessment, target)| {
                    assessment.recipe().as_str() != target.recipe
                        || assessment.target_stack_units() != target.minimum_stack_units
                        || assessment.max_order_amount() != target.max_order_amount
                })
        {
            return Err(denied(
                "no-work stock portfolio differs from its original request",
            ));
        }
        let assessment_witness = binding.witness()?;
        let monitor = StockTargetsWatch::new(
            assessments,
            GameTick(request.deadline_tick),
            request.poll_interval_ticks,
            request.stable_samples,
            context,
        )?;
        Ok(Self {
            binding,
            assessment_witness,
            request,
            monitor,
            cancelled: false,
        })
    }

    pub(super) fn assessment_json(&self) -> Value {
        stock_targets::report(
            &self.binding,
            false,
            self.monitor.high_tick(),
            false,
            None,
            None,
        )
    }
}

impl Runtime {
    pub(super) fn no_work_portfolio_json(&self) -> Value {
        let Some(goal) = &self.stock_targets_no_work else {
            return Value::Null;
        };
        let current = goal.monitor.latest().filter(|sample| {
            !goal.cancelled && !self.clock_withholds_goal() && sample.anchor == self.anchor
        });
        let targets: Vec<_> = goal
            .monitor
            .originals()
            .iter()
            .map(|assessment| {
                json!({
                    "recipe":assessment.recipe().as_str(),
                    "item_type":assessment.item_type(),
                    "target_stack_units":assessment.target_stack_units(),
                    "assessment_witness":assessment.witness().to_hex(),
                })
            })
            .collect();
        json!({
            "schema":"dfmcp.stock-targets-watch/1",
            "scope":"original_no_work_inventory_portfolio",
            "goal_witness":goal.monitor.witness().to_hex(),
            "assessment_witness":goal.assessment_witness.to_hex(),
            "targets":targets,
            "deadline_tick":goal.request.deadline_tick,
            "poll_interval_ticks":goal.request.poll_interval_ticks,
            "stable_samples_required":goal.request.stable_samples,
            "cancelled":goal.cancelled,
            "source_fenced":goal.monitor.fenced(),
            "historical_achieved_at_tick":goal.monitor.achieved_at().map(GameTick::get),
            "consumed_tick_floor":goal.monitor.high_tick().get(),
            "continuation_request":self.stock_continuation_request_json(),
            "original_goal_proven":self.no_work_goal_proven(),
            "current_inventory_proven":self.no_work_goal_proven(),
            "sample":current.map(|sample| json!({
                "anchor":anchor_json(sample.anchor),
                "source_digest":sample.source_digest.to_hex(),
                "source_generation":sample.source_generation,
                "source_counters":sample.source_counters,
                "targets":sample.targets.iter().map(|target| json!({
                    "recipe":target.recipe.as_str(),
                    "item_type":target.item_type,
                    "target_stack_units":target.target_stack_units,
                    "observed_stack_units":target.observed_stack_units,
                    "current_truth":goals::truth(target.current_truth),
                })).collect::<Vec<_>>(),
                "current_truth":goals::truth(sample.current_truth),
                "stable_samples_observed":sample.stable_samples_observed,
                "achieved_at_tick":sample.achieved_at.map(GameTick::get),
                "last_counted_tick":sample.last_counted_tick.map(GameTick::get),
                "deadline_reached":sample.deadline_reached,
                "expired":sample.expired,
                "continuous":sample.continuous,
                "witness":sample.witness.to_hex(),
            })),
            "fresh_for_current_call":false,
            "native_effect_dispatched":false,
            "worker_prerequisite_checked":false,
            "labor_assignment_planned":false,
            "continuous_maintenance":false,
            "production_attribution":"unknown",
            "inventory_semantics":"all_observed_type_stack_units",
            "sample_history_durable":false,
            "recovery_restores_current_proof":false,
        })
    }
}
