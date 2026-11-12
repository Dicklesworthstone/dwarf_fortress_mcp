//! Joint finite stock sizing and receipt-backed queue-prefix checks.
use super::stock_targets_binding::{self, StockTargetsBinding};
use super::*;
use dfmcp_adapter::live_production::SelectedOrderClockCustody;
use dfmcp_adapter::semantic_work_orders::{
    INVENTORY_BASELINE_PROJECTION_BYTES, INVENTORY_BASELINE_QUALIFY_BYTES,
    InventoryBaselineProjection, OperationsGoalSource, SemanticWorkOrderResult,
    StockTargetAssessment,
};
use dfmcp_adapter::work_orders::WorkOrderObservation;
use dfmcp_core::{DfmcpError, ErrorCode, StepId};
use std::sync::{Arc, Mutex};

/// One source read/projection plus four independently qualified target counts.
pub(super) const CENSUS_BYTES: u64 =
    33 * MIB + INVENTORY_BASELINE_PROJECTION_BYTES + 4 * INVENTORY_BASELINE_QUALIFY_BYTES;
pub(super) const CHECK_BYTES: u64 = CENSUS_BYTES + orders::native::STOCK_TARGET_QUEUE_BYTES;
const PREFIX_CUSTODY_BYTES: u64 = 512 * MIB;

fn stale(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::StaleAnchor, message)
}

fn acquire(
    shell: &O,
    targets: &[stock_targets_request::StockTarget],
    consumed: &mut GameTick,
    input: Option<&SharedInputSupply>,
    initial: bool,
    c: &OperationContext,
) -> Result<Vec<StockTargetAssessment>> {
    if targets.is_empty() || targets.len() > 4 {
        return Err(invalid("stock-target census requires one to four targets"));
    }
    if c.anchor.tick < *consumed {
        return Err(stale(
            "reobserve the canonical source after a newer stock-target census",
        ));
    }
    inventory_baseline::authorize_observe(c, *consumed)?;
    shell.check_environment()?;
    let mut work = Work::new(c)?;
    let input_bytes = input.map_or(0, |_| {
        if initial {
            input_supply::CAPTURED_PLAN_BYTES
        } else {
            input_supply::ASSESS_BYTES
        }
    });
    if work.bytes < CENSUS_BYTES + input_bytes {
        return Err(exhausted(
            "complete stock-target portfolio census must be prepaid",
        ));
    }
    let mut source = shell.operations(&work.take(16 * MIB)?).inspect_err(|_| {
        if let Some(input) = input {
            input.native_failed();
        }
    })?;
    let read = work.take(16 * MIB).inspect_err(|_| {
        if let Some(input) = input {
            input.native_failed();
        }
    })?;
    let operations = source.read_operations(&read).inspect_err(|_| {
        if let Some(input) = input {
            input.native_failed();
        }
    })?;
    // Retain decoded input floors immediately, but a fully met portfolio has
    // no input demand and therefore cannot be blocked by an input-only fence.
    let mut input_note_failure = input.and_then(|input| input.note_capture(&operations).err());
    if !initial && let Some(cause) = input_note_failure.take() {
        return Err(cause);
    }
    drop(source);
    *consumed = (*consumed).max(operations.jobs.tick());
    inventory_baseline::authorize_observe(&work.current()?, *consumed)?;
    shell.check_environment()?;
    let projection = InventoryBaselineProjection::new(
        c.anchor,
        operations,
        &work.take(INVENTORY_BASELINE_PROJECTION_BYTES)?,
    )?;
    let policy = shell.acceptance().baseline(&projection, &work.take(MIB)?)?;
    let mut assessments = Vec::with_capacity(targets.len());
    for target in targets {
        assessments.push(projection.stock_target(
            super::request::recipe(&target.recipe)?.0,
            target.minimum_stack_units,
            target.max_order_amount,
            policy.clone(),
            &work.take(INVENTORY_BASELINE_QUALIFY_BYTES)?,
        )?);
    }
    if let Some(input) = input {
        if initial {
            let inputs = PlanInputs::captured(&assessments)?;
            if !inputs.orders.is_empty() {
                if let Some(cause) = input_note_failure.take() {
                    return Err(cause);
                }
                input.prepare(
                    &inputs.orders,
                    Some(projection.operations()),
                    &work.take(input_supply::CAPTURED_PLAN_BYTES)?,
                )?;
            }
        } else {
            input.qualify_captured(
                projection.operations(),
                &work.take(input_supply::ASSESS_BYTES)?,
            )?;
        }
    }
    shell.check_environment()?;
    work.current()?;
    Ok(assessments)
}

#[derive(Clone)]
pub(super) struct ReceiptPrefix {
    session: SessionId,
    request: RequestId,
    step: StepId,
    ids: Vec<u32>,
    after: Option<Digest32>,
}

impl ReceiptPrefix {
    pub(super) fn matches(&self, queue: &WorkOrderObservation, c: &OperationContext) -> bool {
        self.session == c.session_id
            && self.request == c.request_id
            && queue.order_ids() == self.ids
            && self.after.is_none_or(|after| queue.witness() == after)
    }
}

pub(super) fn verified_prefix(
    plan: &PreparedPlan,
    archive: PlanArchiveSeal,
    journal: Digest32,
    step: StepId,
    results: &[SemanticWorkOrderResult],
    c: &OperationContext,
) -> Result<ReceiptPrefix> {
    let layout = live_production::ProductionLayout::from_plan(plan)?;
    let ordinal = layout
        .order_ordinal(step)
        .ok_or_else(|| denied("stock-target prefix selected an original non-order step"))?;
    if c.anchor != plan.anchor || ordinal != results.len() {
        return Err(denied(
            "stock-target prefix differs from the exact original next step",
        ));
    }
    let mut ids = Vec::with_capacity(results.len());
    let mut after = None;
    for (index, result) in results.iter().enumerate() {
        let custody = SelectedOrderClockCustody::new(plan, archive, journal, result)?;
        let before = custody.native_plan().observation();
        if custody.step_id() != layout.order_steps()[index]
            || before.tick() != plan.anchor.tick.get()
            || !before.paused()
            || before.order_ids() != ids
            || after.is_some_and(|after| before.witness() != after)
        {
            return Err(denied(
                "stock-target queue is not the exact original Created prefix",
            ));
        }
        ids.push(custody.native_order_id());
        after = Some(
            result
                .native_record()
                .effect()
                .after_witness()
                .ok_or_else(|| denied("original Created order lacks its exact queue readback"))?,
        );
    }
    Ok(ReceiptPrefix {
        session: c.session_id,
        request: c.request_id,
        step,
        ids,
        after,
    })
}

struct Owner {
    binding: StockTargetsBinding,
    original_digest: Digest32,
    shell: O,
    consumed_tick: GameTick,
    prefix: Option<ReceiptPrefix>,
    revalidation_failed: bool,
    last_checked: Option<Digest32>,
}

#[derive(Clone)]
pub(super) struct SharedStockTargets(Arc<Mutex<Owner>>);

impl SharedStockTargets {
    pub(super) fn from_original(
        plan: &PreparedPlan,
        shell: O,
        authority_floor: GameTick,
    ) -> Result<Option<Self>> {
        let Some(binding) = stock_targets_binding::requirement(plan)? else {
            return Ok(None);
        };
        shell.check_environment()?;
        let policy = shell.acceptance();
        if binding.source_generation != policy.operations_generation
            || binding.original_anchor.fortress_id != policy.fortress()
        {
            return Err(denied(
                "stock targets differ from the independently pinned source",
            ));
        }
        Ok(Some(Self(Arc::new(Mutex::new(Owner {
            binding,
            original_digest: plan.digest,
            shell,
            consumed_tick: plan.anchor.tick.max(authority_floor),
            prefix: None,
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
            owner.prefix = None;
            owner.last_checked = None;
        }
    }

    /// Only actual original Created custody may explain a nonempty queue.
    /// The exact native post-insertion witness includes sequence, horizon and
    /// complete membership, so removing/replacing/adding any row cannot pass.
    fn arm_prefix(
        &self,
        plan: &PreparedPlan,
        archive: PlanArchiveSeal,
        journal: Digest32,
        step: StepId,
        results: &[SemanticWorkOrderResult],
        c: &OperationContext,
    ) -> Result<()> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("stock targets are busy"))?;
        owner.prefix = None;
        owner.last_checked = None;
        inventory_baseline::authorize_observe(c, owner.consumed_tick)?;
        if plan.digest != owner.original_digest || plan.anchor != owner.binding.original_anchor {
            return Err(denied("stock-target prefix differs from its original plan"));
        }
        owner.prefix = Some(verified_prefix(plan, archive, journal, step, results, c)?);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn qualify(&self, queue: &WorkOrderObservation, c: &OperationContext) -> Result<()> {
        self.qualify_with_input(queue, None, c)
    }
    pub(super) fn qualify_with_input(
        &self,
        queue: &WorkOrderObservation,
        input: Option<&SharedInputSupply>,
        c: &OperationContext,
    ) -> Result<()> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("stock targets are busy"))?;
        owner.last_checked = None;
        owner.consumed_tick = owner.consumed_tick.max(GameTick(queue.tick()));
        let result = owner.qualify(queue, input, c);
        owner.revalidation_failed = result.is_err();
        if result.is_err() {
            if let Some(input) = input {
                input.guard_failed();
            }
        }
        result
    }

    pub(super) fn status(&self) -> Value {
        let Ok(owner) = self.0.try_lock() else {
            return json!({"schema":"dfmcp.stock-targets/1","status":"unavailable",
                "fresh_for_current_call":false,"current_inventory_proven":false});
        };
        report(
            &owner.binding,
            true,
            owner.consumed_tick,
            owner.revalidation_failed,
            owner.last_checked,
            owner.prefix.as_ref().map(|prefix| prefix.step),
        )
    }
}

impl Owner {
    fn qualify(
        &mut self,
        queue: &WorkOrderObservation,
        input: Option<&SharedInputSupply>,
        c: &OperationContext,
    ) -> Result<()> {
        c.budget.validate()?;
        let input_bytes = input.map_or(0, |_| input_supply::GUARD_BYTES);
        if c.budget.max_bytes < CHECK_BYTES + input_bytes {
            return Err(exhausted(
                "stock-target portfolio census and queue checks must be prepaid",
            ));
        }
        if let Some(input) = input {
            input.begin_guard(queue, c)?;
        }
        inventory_baseline::authorize_observe(c, self.consumed_tick)?;
        self.shell.check_environment()?;
        let original = self.binding.original_anchor;
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
                "stock targets require their exact original paused source tick",
            ));
        }
        if self
            .prefix
            .as_ref()
            .is_none_or(|prefix| !prefix.matches(queue, c))
        {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "stock targets require exactly the receipt-backed original order prefix",
            ));
        }
        let targets: Vec<_> = self
            .binding
            .rows
            .iter()
            .map(|row| stock_targets_request::StockTarget {
                recipe: row.recipe.as_str().into(),
                minimum_stack_units: row.target,
                max_order_amount: row.maximum,
            })
            .collect();
        let mut current = c.clone();
        current.anchor = original;
        let mut work = Work::new(&current)?;
        let assessments = acquire(
            &self.shell,
            &targets,
            &mut self.consumed_tick,
            input,
            false,
            &work.take(CENSUS_BYTES + input.map_or(0, |_| input_supply::ASSESS_BYTES))?,
        )?;
        let fresh = if self.binding.assign_labor() {
            StockTargetsBinding::from_assessments_with_assignment(&assessments)?
        } else {
            StockTargetsBinding::from_assessments(&assessments)?
        };
        if fresh != self.binding {
            return Err(stale(
                "stock-target census changed; retain every original amount and minimum",
            ));
        }
        let fresh_queue = self.shell.stock_target_queue(
            &work.take(orders::native::STOCK_TARGET_QUEUE_BYTES)?,
            &mut self.consumed_tick,
        )?;
        if fresh_queue != *queue
            || self
                .prefix
                .as_ref()
                .is_none_or(|prefix| !prefix.matches(&fresh_queue, c))
        {
            return Err(stale(
                "stock-target queue changed after census; retain the original keys",
            ));
        }
        if let Some(input) = input {
            input.finish_guard(&fresh_queue, &work.current()?)?;
        }
        self.shell.check_environment()?;
        work.current()?;
        self.last_checked = Some(fresh.witness()?);
        Ok(())
    }
}

fn report(
    binding: &StockTargetsBinding,
    planned: bool,
    floor: GameTick,
    failed: bool,
    checked: Option<Digest32>,
    next_step: Option<StepId>,
) -> Value {
    let assign_labor = binding.assign_labor();
    let mut ordinal = u32::from(assign_labor);
    let rows: Vec<_> = binding
        .rows
        .iter()
        .map(|row| {
            let step = if row.deficit > 0 {
                let step = ordinal;
                ordinal += 1;
                Some(step)
            } else {
                None
            };
            json!({"schema":"dfmcp.stock-target/1",
            "status":if row.deficit>0 {"planned"} else {"already_satisfied"},
            "recipe":row.recipe.as_str(),"item_type":row.item_type(),
            "baseline_stack_units":row.baseline,"target_stack_units":row.target,
            "deficit_stack_units":row.deficit,"amount":row.deficit,
            "max_order_amount":row.maximum,"assessment_witness":row.witness.to_hex(),
            "baseline_anchor":anchor_json(binding.original_anchor),
            "source_digest":binding.source_digest.to_hex(),
            "source_generation":binding.source_generation,"source_counters":binding.source_counters,
            "item_rows":binding.item_rows,"order_step_id":step,
            "work_required":row.deficit>0,"mutation_planned":planned && row.deficit>0,
            "satisfied_at_anchor":row.deficit==0,"current_inventory_proven":false,
            "fresh_for_current_call":false,
            "labor_assignment_requested":assign_labor,
            "labor_assignment_planned":planned && assign_labor && row.deficit>0})
        })
        .collect();
    json!({"schema":"dfmcp.stock-targets/1",
        "status":if planned {"planned"} else {"already_satisfied"},
        "targets":rows,"baseline_anchor":anchor_json(binding.original_anchor),
        "source_digest":binding.source_digest.to_hex(),"source_generation":binding.source_generation,
        "source_counters":binding.source_counters,"item_rows":binding.item_rows,
        "assessment_witness":binding.witness().ok().map(|digest|digest.to_hex()),
        "consumed_tick_floor":floor.get(),"revalidation_failed":failed,
        "last_check_witness":checked.map(|digest|digest.to_hex()),
        "last_check_order_step_id":checked.and(next_step).map(|step|step.get()),
        "work_required":planned,"mutation_planned":planned,
        "worker_prerequisite_checked":planned && !assign_labor,
        "satisfied_at_anchor":!planned,
        "labor_assignment_requested":assign_labor,"labor_assignment_planned":planned && assign_labor,
        "exact_original_receipt_prefix_required_before_insertion":true,
        "exact_original_census_required_before_insertion":true,
        "fresh_for_current_call":false,"current_inventory_proven":false,
        "all_target_minima_retained":true,"continuous_maintenance":false,
        "native_recipe_feasibility_proven":false,"production_attribution":"unknown",
        "inventory_semantics":"all_observed_type_stack_units"})
}

impl Runtime {
    pub(crate) fn plan_stock_targets(
        &mut self,
        request: StockTargetsRequest,
        c: &OperationContext,
    ) -> Result<View> {
        request.validate()?;
        self.check(c, true)?;
        self.stock_targets_no_work = None;
        self.stock_target_no_work = None;
        self.planning_inspection.withdraw();
        if self.original.is_some() {
            return Err(invalid(
                "retain the original stock targets; never recalculate their amounts",
            ));
        }
        let span = request
            .deadline_tick
            .checked_sub(c.anchor.tick.get())
            .filter(|span| {
                *span > 0 && *span <= dfmcp_intent::effects::MAX_DEFAULT_OBLIGATION_TICKS
            })
            .ok_or_else(|| {
                invalid("stock-target deadline must be future and within one game year")
            })?;
        if request
            .poll_interval_ticks
            .checked_mul(u64::from(request.stable_samples))
            .is_none_or(|duration| duration > span)
        {
            return Err(invalid(
                "stock-target deadline cannot contain its cadence and stability",
            ));
        }
        let basis = self
            .basis
            .as_ref()
            .ok_or_else(|| invalid("observe selected workers before stock-target planning"))?;
        if request.observation()? != basis.seal {
            return Err(denied(
                "stock-target portfolio names another original observation",
            ));
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
                "stock-target workers are outside the original canonical roster",
            ));
        }
        let mut work = Work::new(c)?;
        let assessments = acquire(
            &self.config.orders,
            &request.targets,
            &mut self.observation_tick_floor,
            self.input_supply.as_ref(),
            true,
            &work.take(
                CENSUS_BYTES
                    + self
                        .input_supply
                        .as_ref()
                        .map_or(0, |_| input_supply::CAPTURED_PLAN_BYTES),
            )?,
        )?;
        let binding = if request.assign_labor {
            StockTargetsBinding::from_assessments_with_assignment(&assessments)?
        } else {
            StockTargetsBinding::from_assessments(&assessments)?
        };
        if assessments
            .iter()
            .all(|assessment| assessment.amount().is_none())
        {
            self.stock_targets_no_work = Some(report(
                &binding,
                false,
                self.observation_tick_floor,
                false,
                None,
                None,
            ));
            return self.view(&mut work);
        }
        if request.assign_labor {
            // No-work has already returned. A real assignment requires current
            // effect authority without turning the Plan call into a confirmation.
            let mut authority = work.current()?;
            authority.anchor.tick = authority.anchor.tick.max(self.authority_tick_floor());
            authority.authorize(
                Capability::ConfigureLabor,
                RiskTier::Guarded,
                &request.unit_ids()?,
                None,
            )?;
            authority.authorize(
                Capability::ConfigureProduction,
                RiskTier::Guarded,
                &[],
                None,
            )?;
            return self.plan_compiled(
                request.observation()?,
                true,
                &[],
                PlanInputs::captured(&assessments)?,
                &work.current()?,
                |evidence, _, current, origin| {
                    request.compile_with_labor_assignment(&assessments, evidence, current, origin)
                },
            );
        }
        self.plan_existing_workers(
            request.observation()?,
            (request.unit_ids()?, request.labor.clone()),
            true,
            &[],
            PlanInputs::captured(&assessments)?,
            &work.current()?,
            |evidence, workers, _, current, origin| {
                request.compile(&assessments, evidence, workers, current, origin)
            },
        )
    }

    /// Refresh historical receipt custody in this exact operation before the
    /// native guard can accept a nonempty prefix. No prior call's selection is
    /// reused, and this read preserves the separately prepared current order.
    pub(super) fn arm_stock_targets_prefix(&mut self, work: &mut Work) -> Result<()> {
        let targets = self.stock_targets.clone();
        let inputs = self.input_supply.clone();
        if targets.is_none() && inputs.is_none() {
            return Ok(());
        }
        if let Some(targets) = &targets {
            targets.withdraw();
        }
        if let Some(inputs) = &inputs {
            inputs.withdraw();
        }
        let view = self
            .frontier
            .as_mut()
            .ok_or_else(|| invalid("stock-target frontier is absent"))?
            .inspect(&work.take(8 * MIB)?)?;
        let selected = view
            .entries
            .iter()
            .find(|entry| entry.phase == Phase::Prepared)
            .map(|entry| entry.step)
            .or(view.next_ready);
        let Some(selected) = selected else {
            return Ok(());
        };
        let plan = self
            .original
            .as_ref()
            .ok_or_else(|| invalid("stock-target original is absent"))?;
        let layout = self
            .layout
            .as_ref()
            .ok_or_else(|| invalid("stock-target original layout is absent"))?;
        if layout.assignment_step() == Some(selected) {
            // The insertion-only guard must not turn a labor preparation into
            // an order-census read or demand an already-applied assignment.
            return Ok(());
        }
        let ordinal = layout
            .order_ordinal(selected)
            .ok_or_else(|| invalid("stock-target next original step is not an order"))?;
        let archive = self
            .seal
            .ok_or_else(|| invalid("stock-target archive is absent"))?;
        let mut results = Vec::with_capacity(ordinal);
        for step in layout.order_steps().iter().copied().take(ordinal) {
            let entry = view
                .entries
                .iter()
                .find(|entry| entry.step == step)
                .filter(|entry| entry.phase == Phase::Applied)
                .ok_or_else(|| {
                    denied("stock-target predecessor is not an original Applied effect")
                })?;
            let result = self
                .execution
                .as_mut()
                .ok_or_else(|| invalid("stock-target executor is absent"))?
                .inspect_retained_order_custody(step, &work.take(PREFIX_CUSTODY_BYTES)?)?;
            let native_receipt = result
                .native_record()
                .effect()
                .receipt()
                .ok_or_else(|| denied("stock-target predecessor lacks its Created receipt"))?;
            let mut bytes = b"dfmcp-live-production-family-receipt/1\0".to_vec();
            bytes.extend_from_slice(b"work-orders/1.10");
            bytes.extend_from_slice(result.review().seal().as_bytes());
            bytes.extend_from_slice(native_receipt.as_bytes());
            if entry.receipt.is_none_or(|receipt| {
                receipt.family != dfmcp_adapter::live_routing::LiveFamily::WorkOrdersV1_10
                    || receipt.digest != Digest32::of_bytes(&bytes)
            }) {
                return Err(denied(
                    "stock-target frontier differs from its actual Created receipt",
                ));
            }
            results.push(result);
        }
        if let Some(targets) = targets {
            targets.arm_prefix(
                plan,
                archive,
                self.order_journal,
                selected,
                &results,
                &work.current()?,
            )?;
        }
        if let Some(inputs) = inputs {
            inputs.arm_prefix(
                plan,
                archive,
                self.order_journal,
                selected,
                &results,
                &work.current()?,
            )?;
        }
        Ok(())
    }

    pub(super) fn stock_targets_json(&self) -> Value {
        self.stock_targets.as_ref().map_or_else(
            || self.stock_targets_no_work.clone().unwrap_or(Value::Null),
            SharedStockTargets::status,
        )
    }
}

#[cfg(test)]
#[path = "stock_targets_guard_tests.rs"]
mod tests;
