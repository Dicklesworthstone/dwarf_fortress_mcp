//! Fresh, bounded stock sizing and the original census guard at native edges.
//! A stock target is one finite order, never an unattended maintenance policy.
use super::stock_target_binding::{self, StockTargetBinding};
use super::*;
use dfmcp_adapter::semantic_work_orders::{
    INVENTORY_BASELINE_PROJECTION_BYTES, INVENTORY_BASELINE_QUALIFY_BYTES,
    InventoryBaselineProjection, OperationsGoalSource, StockTargetAssessment,
};
use dfmcp_adapter::work_orders::{WorkOrderObservation, WorkOrderRecipe};
use dfmcp_core::{DfmcpError, ErrorCode};
use std::sync::{Arc, Mutex};

/// One connection and complete read, one projection, one independent source
/// policy and one qualified target. Reserve it before credentials or contact.
pub(super) const CENSUS_BYTES: u64 =
    33 * MIB + INVENTORY_BASELINE_PROJECTION_BYTES + INVENTORY_BASELINE_QUALIFY_BYTES;
pub(super) const CHECK_BYTES: u64 = CENSUS_BYTES + orders::native::STOCK_TARGET_QUEUE_BYTES;

fn stale(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::StaleAnchor, message)
}

fn acquire(
    shell: &O,
    recipe: WorkOrderRecipe,
    target: u64,
    maximum: u32,
    consumed: &mut GameTick,
    c: &OperationContext,
) -> Result<StockTargetAssessment> {
    if c.anchor.tick < *consumed {
        return Err(stale(
            "reobserve the canonical source after a newer stock census",
        ));
    }
    inventory_baseline::authorize_observe(c, *consumed)?;
    shell.check_environment()?;
    let mut work = Work::new(c)?;
    if work.bytes < CENSUS_BYTES {
        return Err(exhausted(
            "complete stock-target read exceeds its reserved allowance",
        ));
    }
    let mut source = shell.operations(&work.take(16 * MIB)?)?;
    let operations = source.read_operations(&work.take(16 * MIB)?)?;
    drop(source);
    // Even a later pin/tick/census refusal cannot revive an expired grant.
    *consumed = (*consumed).max(operations.jobs.tick());
    inventory_baseline::authorize_observe(&work.current()?, *consumed)?;
    shell.check_environment()?;
    let projection = InventoryBaselineProjection::new(
        c.anchor,
        operations,
        &work.take(INVENTORY_BASELINE_PROJECTION_BYTES)?,
    )?;
    let policy = shell.acceptance().baseline(&projection, &work.take(MIB)?)?;
    let assessment = projection.stock_target(
        recipe,
        target,
        maximum,
        policy,
        &work.take(INVENTORY_BASELINE_QUALIFY_BYTES)?,
    )?;
    shell.check_environment()?;
    work.current()?;
    Ok(assessment)
}

struct Owner {
    binding: StockTargetBinding,
    shell: O,
    consumed_tick: GameTick,
    // This records a failed dispatch-basis check in this owner. Reopening does
    // not turn it into fresh evidence: every possible insertion must reread the
    // exact original census again. No independent durable read ACK is claimed.
    revalidation_failed: bool,
    last_checked: Option<Digest32>,
}

#[derive(Clone)]
pub(super) struct SharedStockTarget(Arc<Mutex<Owner>>);

impl SharedStockTarget {
    pub(super) fn from_original(
        plan: &PreparedPlan,
        shell: O,
        authority_floor: GameTick,
    ) -> Result<Option<Self>> {
        let Some(binding) = stock_target_binding::requirement(plan)? else {
            return Ok(None);
        };
        shell.check_environment()?;
        let policy = shell.acceptance();
        if binding.objective().source_generation() != policy.operations_generation
            || binding.objective().baseline_anchor().fortress_id != policy.fortress()
        {
            return Err(denied(
                "stock target differs from the independently pinned source",
            ));
        }
        Ok(Some(Self(Arc::new(Mutex::new(Owner {
            consumed_tick: plan.anchor.tick.max(authority_floor),
            binding,
            shell,
            revalidation_failed: false,
            last_checked: None,
        })))))
    }

    pub(super) fn high_tick(&self) -> GameTick {
        self.0
            .try_lock()
            .map_or(GameTick(u64::MAX), |owner| owner.consumed_tick)
    }

    pub(super) fn withdraw(&self) {
        if let Ok(mut owner) = self.0.try_lock() {
            owner.last_checked = None;
        }
    }

    pub(super) fn qualify(&self, queue: &WorkOrderObservation, c: &OperationContext) -> Result<()> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("stock target is busy"))?;
        owner.last_checked = None;
        // Retain decoded time before any fallible authority or source check.
        owner.consumed_tick = owner.consumed_tick.max(GameTick(queue.tick()));
        let result = owner.qualify(queue, c);
        owner.revalidation_failed = result.is_err();
        result
    }

    pub(super) fn status(&self) -> Value {
        let Ok(owner) = self.0.try_lock() else {
            return json!({"schema":"dfmcp.stock-target/1","status":"unavailable",
                "fresh_for_current_call":false,"current_inventory_proven":false});
        };
        let objective = owner.binding.objective();
        json!({"schema":"dfmcp.stock-target/1","status":"planned",
            "recipe":owner.binding.recipe().as_str(),"item_type":objective.item_type(),
            "baseline_stack_units":objective.baseline_stack_units(),
            "target_stack_units":objective.target_stack_units(),
            "deficit_stack_units":objective.increase_stack_units(),
            "amount":objective.increase_stack_units(),
            "max_order_amount":owner.binding.max_order_amount(),
            "baseline_anchor":anchor_json(objective.baseline_anchor()),
            "source_digest":objective.baseline_source_digest().to_hex(),
            "source_generation":objective.source_generation(),
            "source_counters":objective.baseline_source_counters(),
            "assessment_witness":owner.binding.witness().to_hex(),
            "consumed_tick_floor":owner.consumed_tick.get(),
            "revalidation_failed":owner.revalidation_failed,
            "last_check_witness":owner.last_checked.map(|digest|digest.to_hex()),
            "work_required":true,"mutation_planned":true,
            "labor_assignment_requested":owner.binding.assign_labor(),
            "labor_assignment_planned":owner.binding.assign_labor(),
            "order_step_id":if owner.binding.assign_labor() {1}else{0},
            "worker_prerequisite_checked":!owner.binding.assign_labor(),
            "empty_queue_required_before_insertion":true,
            "exact_original_census_required_before_insertion":true,
            "fresh_for_current_call":false,"current_inventory_proven":false,
            "continuous_maintenance":false,"native_recipe_feasibility_proven":false,
            "production_attribution":"unknown","inventory_semantics":"all_observed_type_stack_units"})
    }
}

impl Owner {
    fn qualify(&mut self, queue: &WorkOrderObservation, c: &OperationContext) -> Result<()> {
        c.budget.validate()?;
        if c.budget.max_bytes < CHECK_BYTES {
            return Err(exhausted(
                "stock-target census and queue checks must be prepaid together",
            ));
        }
        inventory_baseline::authorize_observe(c, self.consumed_tick)?;
        self.shell.check_environment()?;
        let original = self.binding.objective().baseline_anchor();
        let policy = self.shell.acceptance();
        if c.anchor.fortress_id != original.fortress_id
            || c.anchor.cursor.epoch != original.cursor.epoch
            || c.anchor.tick != original.tick
            || queue.fortress_id() != original.fortress_id
            || queue.world_folder() != policy.world_folder
            || queue.site_id() != policy.site_id
            || queue.generation() != policy.work_order_generation
            || !queue.paused()
            || queue.tick() != original.tick.get()
        {
            return Err(stale(
                "stock target requires its exact original paused source tick",
            ));
        }
        if !queue.order_ids().is_empty() {
            // Protocol 1.10 proves membership only, not recipe or remaining
            // quantities. Refuse all queued orders instead of assuming that an
            // unclassified row cannot produce this output or crediting it.
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "stock target requires an empty manager-order queue before insertion",
            ));
        }
        let mut current = c.clone();
        // The fresh read must independently have this tick. This local context
        // selects the immutable census; it publishes no new canonical anchor.
        current.anchor = original;
        let mut work = Work::new(&current)?;
        let assessment = acquire(
            &self.shell,
            self.binding.recipe(),
            self.binding.objective().target_stack_units(),
            self.binding.max_order_amount(),
            &mut self.consumed_tick,
            &work.take(CENSUS_BYTES)?,
        )?;
        if assessment.witness() != self.binding.witness()
            || assessment.objective() != Some(self.binding.objective())
        {
            return Err(stale(
                "stock target census changed; retain the original and do not resize its order",
            ));
        }
        // This source is acquired after the fresh inventory read. The queue
        // passed by the existing native guard may be the immutable original
        // observation; comparing it with another real read closes that gap.
        let fresh_queue = self.shell.stock_target_queue(
            &work.take(orders::native::STOCK_TARGET_QUEUE_BYTES)?,
            &mut self.consumed_tick,
        )?;
        if fresh_queue != *queue || !fresh_queue.order_ids().is_empty() {
            return Err(stale(
                "stock target native queue changed; retain its original preparation and key",
            ));
        }
        self.shell.check_environment()?;
        work.current()?;
        self.last_checked = Some(assessment.witness());
        Ok(())
    }
}

fn no_work(assessment: &StockTargetAssessment, assign_labor: bool) -> Value {
    json!({"schema":"dfmcp.stock-target/1","status":"already_satisfied",
        "recipe":assessment.recipe().as_str(),"item_type":assessment.item_type(),
        "baseline_stack_units":assessment.baseline_stack_units(),
        "target_stack_units":assessment.target_stack_units(),
        "deficit_stack_units":0,"amount":0,"max_order_amount":assessment.max_order_amount(),
        "baseline_anchor":anchor_json(assessment.original_anchor()),
        "source_digest":assessment.source_digest().to_hex(),
        "source_generation":assessment.source_generation(),
        "source_counters":assessment.source_counters(),"item_rows":assessment.item_rows(),
        "assessment_witness":assessment.witness().to_hex(),
        "work_required":false,"mutation_planned":false,"satisfied_at_anchor":true,
        "labor_assignment_requested":assign_labor,"labor_assignment_planned":false,
        "order_step_id":null,"worker_prerequisite_checked":false,
        "fresh_for_current_call":false,"current_inventory_proven":false,
        "continuous_maintenance":false,"native_recipe_feasibility_proven":false,
        "production_attribution":"unknown","inventory_semantics":"all_observed_type_stack_units"})
}

impl Runtime {
    pub(crate) fn plan_stock_target(
        &mut self,
        request: StockTargetRequest,
        c: &OperationContext,
    ) -> Result<View> {
        request.validate()?;
        self.check(c, true)?;
        self.stock_target_no_work = None;
        self.planning_inspection.withdraw();
        if self.original.is_some() {
            return Err(invalid(
                "retain the original stock target; never recalculate its amount",
            ));
        }
        let span = request
            .deadline_tick
            .checked_sub(c.anchor.tick.get())
            .filter(|span| {
                *span > 0 && *span <= dfmcp_intent::effects::MAX_DEFAULT_OBLIGATION_TICKS
            })
            .ok_or_else(|| {
                invalid("stock target deadline must be future and at most one game year away")
            })?;
        if request
            .poll_interval_ticks
            .checked_mul(u64::from(request.stable_samples))
            .is_none_or(|duration| duration > span)
        {
            return Err(invalid(
                "stock target deadline cannot contain its cadence and stability",
            ));
        }
        let basis = self
            .basis
            .as_ref()
            .ok_or_else(|| invalid("observe selected workers before stock-target planning"))?;
        if request.observation()? != basis.seal {
            return Err(denied("stock target names another original observation"));
        }
        if request.unit_ids()?.iter().any(|id| {
            basis
                .projection
                .snapshot
                .graph
                .entities
                .get(id)
                .is_none_or(|unit| unit.kind != dfmcp_world::EntityKind::Unit)
        }) {
            return Err(invalid(
                "stock target selected a citizen outside the exact canonical roster",
            ));
        }
        let mut work = Work::new(c)?;
        let assessment = acquire(
            &self.config.orders,
            super::request::recipe(&request.recipe)?.0,
            request.minimum_stack_units,
            request.max_order_amount,
            &mut self.observation_tick_floor,
            &work.take(CENSUS_BYTES)?,
        )?;
        if assessment.amount().is_none() {
            self.stock_target_no_work = Some(no_work(&assessment, request.assign_labor));
            return self.view(&mut work);
        }
        if request.assign_labor {
            // A positive assigned target must be runnable under current grants.
            // Check only after no-work: a met target needs no effect authority.
            let mut authority = work.current()?;
            authority.anchor.tick = authority.anchor.tick.max(self.authority_tick_floor());
            authority.authorize(
                Capability::ConfigureLabor,
                RiskTier::Guarded,
                &request.unit_ids()?,
                None,
            )?;
            authority.authorize(Capability::ConfigureProduction, RiskTier::Guarded, &[], None)?;
            // The qualified assessment already owns the one inventory baseline.
            // Reuse the ordinary assignment compiler and durable two-step owner;
            // neither acquire a second baseline nor bypass native labor review.
            return self.plan_compiled(
                request.observation()?,
                false,
                &[],
                &work.current()?,
                |evidence, _, current, origin| {
                    request.compile_with_labor_assignment(&assessment, evidence, current, origin)
                },
            );
        }
        self.plan_existing_workers(
            request.observation()?,
            (request.unit_ids()?, request.labor.clone()),
            false,
            &[],
            &work.current()?,
            |evidence, requirement, _, current, origin| {
                request.compile(&assessment, evidence, requirement, current, origin)
            },
        )
    }

    pub(super) fn stock_target_json(&self) -> Value {
        self.stock_target.as_ref().map_or_else(
            || self.stock_target_no_work.clone().unwrap_or(Value::Null),
            SharedStockTarget::status,
        )
    }
}
