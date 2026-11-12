//! Joint finite stock sizing and receipt-backed queue-prefix checks.
use super::stock_targets_binding::{self, StockTargetsBinding};
use super::*;
use dfmcp_adapter::live_production::SelectedOrderClockCustody;
use dfmcp_adapter::semantic_work_orders::{
    INVENTORY_BASELINE_PROJECTION_BYTES, INVENTORY_BASELINE_QUALIFY_BYTES,
    InventoryBaselineProjection, OperationsGoalSource, SemanticWorkOrderResult, StockTargetAssessment,
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
    c: &OperationContext,
) -> Result<Vec<StockTargetAssessment>> {
    if targets.is_empty() || targets.len() > 4 {
        return Err(invalid("stock-target census requires one to four targets"));
    }
    if c.anchor.tick < *consumed {
        return Err(stale("reobserve the canonical source after a newer stock-target census"));
    }
    inventory_baseline::authorize_observe(c, *consumed)?;
    shell.check_environment()?;
    let mut work = Work::new(c)?;
    if work.bytes < CENSUS_BYTES {
        return Err(exhausted("complete stock-target portfolio census must be prepaid"));
    }
    let mut source = shell.operations(&work.take(16 * MIB)?)?;
    let operations = source.read_operations(&work.take(16 * MIB)?)?;
    drop(source);
    *consumed = (*consumed).max(operations.jobs.tick());
    inventory_baseline::authorize_observe(&work.current()?, *consumed)?;
    shell.check_environment()?;
    let projection = InventoryBaselineProjection::new(
        c.anchor, operations, &work.take(INVENTORY_BASELINE_PROJECTION_BYTES)?,
    )?;
    let policy = shell.acceptance().baseline(&projection, &work.take(MIB)?)?;
    let mut assessments = Vec::with_capacity(targets.len());
    for target in targets {
        assessments.push(projection.stock_target(
            super::request::recipe(&target.recipe)?.0,
            target.minimum_stack_units, target.max_order_amount,
            policy.clone(), &work.take(INVENTORY_BASELINE_QUALIFY_BYTES)?,
        )?);
    }
    shell.check_environment()?;
    work.current()?;
    Ok(assessments)
}

#[derive(Clone)]
struct ReceiptPrefix {
    session: SessionId,
    request: RequestId,
    step: StepId,
    ids: Vec<u32>,
    after: Option<Digest32>,
}

impl ReceiptPrefix {
    fn matches(&self, queue: &WorkOrderObservation, c: &OperationContext) -> bool {
        self.session == c.session_id && self.request == c.request_id
            && queue.order_ids() == self.ids
            && self.after.is_none_or(|after| queue.witness() == after)
    }
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
        plan: &PreparedPlan, shell: O, authority_floor: GameTick,
    ) -> Result<Option<Self>> {
        let Some(binding) = stock_targets_binding::requirement(plan)? else {
            return Ok(None);
        };
        shell.check_environment()?;
        let policy = shell.acceptance();
        if binding.source_generation != policy.operations_generation
            || binding.original_anchor.fortress_id != policy.fortress()
        {
            return Err(denied("stock targets differ from the independently pinned source"));
        }
        Ok(Some(Self(Arc::new(Mutex::new(Owner {
            binding, original_digest: plan.digest, shell,
            consumed_tick: plan.anchor.tick.max(authority_floor),
            prefix: None, revalidation_failed: false, last_checked: None,
        })))))
    }

    pub(super) fn high_tick(&self) -> GameTick {
        self.0.try_lock().map_or(GameTick(u64::MAX), |owner| owner.consumed_tick)
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
        &self, plan: &PreparedPlan, archive: PlanArchiveSeal, journal: Digest32,
        step: StepId, results: &[SemanticWorkOrderResult], c: &OperationContext,
    ) -> Result<()> {
        let mut owner = self.0.try_lock().map_err(|_| denied("stock targets are busy"))?;
        owner.prefix = None;
        owner.last_checked = None;
        inventory_baseline::authorize_observe(c, owner.consumed_tick)?;
        if plan.digest != owner.original_digest || plan.anchor != owner.binding.original_anchor
            || c.anchor != plan.anchor || step.get() as usize != results.len()
            || step.get() as usize >= plan.steps.len()
        {
            return Err(denied("stock-target prefix differs from the exact original next step"));
        }
        let mut ids = Vec::with_capacity(results.len());
        let mut after = None;
        for (index, result) in results.iter().enumerate() {
            let custody = SelectedOrderClockCustody::new(plan, archive, journal, result)?;
            let before = custody.native_plan().observation();
            if custody.step_id() != StepId::new(index as u32)
                || before.tick() != plan.anchor.tick.get() || !before.paused()
                || before.order_ids() != ids
                || after.is_some_and(|after| before.witness() != after)
            {
                return Err(denied("stock-target queue is not the exact original Created prefix"));
            }
            ids.push(custody.native_order_id());
            after = Some(result.native_record().effect().after_witness()
                .ok_or_else(|| denied("original Created order lacks its exact queue readback"))?);
        }
        owner.prefix = Some(ReceiptPrefix {
            session: c.session_id, request: c.request_id, step, ids, after,
        });
        Ok(())
    }

    pub(super) fn qualify(&self, queue: &WorkOrderObservation, c: &OperationContext) -> Result<()> {
        let mut owner = self.0.try_lock().map_err(|_| denied("stock targets are busy"))?;
        owner.last_checked = None;
        owner.consumed_tick = owner.consumed_tick.max(GameTick(queue.tick()));
        let result = owner.qualify(queue, c);
        owner.revalidation_failed = result.is_err();
        result
    }

    pub(super) fn status(&self) -> Value {
        let Ok(owner) = self.0.try_lock() else {
            return json!({"schema":"dfmcp.stock-targets/1","status":"unavailable",
                "fresh_for_current_call":false,"current_inventory_proven":false});
        };
        report(&owner.binding, true, owner.consumed_tick, owner.revalidation_failed,
            owner.last_checked, owner.prefix.as_ref().map(|prefix| prefix.step))
    }
}

impl Owner {
    fn qualify(&mut self, queue: &WorkOrderObservation, c: &OperationContext) -> Result<()> {
        c.budget.validate()?;
        if c.budget.max_bytes < CHECK_BYTES {
            return Err(exhausted("stock-target portfolio census and queue checks must be prepaid"));
        }
        inventory_baseline::authorize_observe(c, self.consumed_tick)?;
        self.shell.check_environment()?;
        let original = self.binding.original_anchor;
        let policy = self.shell.acceptance();
        if c.anchor.fortress_id != original.fortress_id
            || c.anchor.cursor.epoch != original.cursor.epoch || c.anchor.tick != original.tick
            || queue.fortress_id() != original.fortress_id
            || queue.world_folder() != policy.world_folder || queue.site_id() != policy.site_id
            || queue.generation() != policy.work_order_generation
            || !queue.paused() || queue.tick() != original.tick.get()
        {
            return Err(stale("stock targets require their exact original paused source tick"));
        }
        if self.prefix.as_ref().is_none_or(|prefix| !prefix.matches(queue, c)) {
            return Err(DfmcpError::new(ErrorCode::Conflict,
                "stock targets require exactly the receipt-backed original order prefix"));
        }
        let targets: Vec<_> = self.binding.rows.iter().map(|row| stock_targets_request::StockTarget {
            recipe: row.recipe.as_str().into(),
            minimum_stack_units: row.target,
            max_order_amount: row.maximum,
        }).collect();
        let mut current = c.clone();
        current.anchor = original;
        let mut work = Work::new(&current)?;
        let assessments = acquire(
            &self.shell, &targets, &mut self.consumed_tick, &work.take(CENSUS_BYTES)?,
        )?;
        let fresh = StockTargetsBinding::from_assessments(&assessments)?;
        if fresh != self.binding {
            return Err(stale("stock-target census changed; retain every original amount and minimum"));
        }
        let fresh_queue = self.shell.stock_target_queue(
            &work.take(orders::native::STOCK_TARGET_QUEUE_BYTES)?, &mut self.consumed_tick,
        )?;
        if fresh_queue != *queue
            || self.prefix.as_ref().is_none_or(|prefix| !prefix.matches(&fresh_queue, c))
        {
            return Err(stale("stock-target queue changed after census; retain the original keys"));
        }
        self.shell.check_environment()?;
        work.current()?;
        self.last_checked = Some(fresh.witness()?);
        Ok(())
    }
}

fn report(
    binding: &StockTargetsBinding, planned: bool, floor: GameTick, failed: bool,
    checked: Option<Digest32>, next_step: Option<StepId>,
) -> Value {
    let mut ordinal = 0_u32;
    let rows: Vec<_> = binding.rows.iter().map(|row| {
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
            "labor_assignment_requested":false,"labor_assignment_planned":false})
    }).collect();
    json!({"schema":"dfmcp.stock-targets/1",
        "status":if planned {"planned"} else {"already_satisfied"},
        "targets":rows,"baseline_anchor":anchor_json(binding.original_anchor),
        "source_digest":binding.source_digest.to_hex(),"source_generation":binding.source_generation,
        "source_counters":binding.source_counters,"item_rows":binding.item_rows,
        "assessment_witness":binding.witness().ok().map(|digest|digest.to_hex()),
        "consumed_tick_floor":floor.get(),"revalidation_failed":failed,
        "last_check_witness":checked.map(|digest|digest.to_hex()),
        "last_check_order_step_id":checked.and(next_step).map(|step|step.get()),
        "work_required":planned,"mutation_planned":planned,"worker_prerequisite_checked":planned,
        "satisfied_at_anchor":!planned,
        "labor_assignment_requested":false,"labor_assignment_planned":false,
        "exact_original_receipt_prefix_required_before_insertion":true,
        "exact_original_census_required_before_insertion":true,
        "fresh_for_current_call":false,"current_inventory_proven":false,
        "all_target_minima_retained":true,"continuous_maintenance":false,
        "native_recipe_feasibility_proven":false,"production_attribution":"unknown",
        "inventory_semantics":"all_observed_type_stack_units"})
}

impl Runtime {
    pub(crate) fn plan_stock_targets(
        &mut self, request: StockTargetsRequest, c: &OperationContext,
    ) -> Result<View> {
        request.validate()?;
        self.check(c, true)?;
        self.stock_targets_no_work = None;
        self.stock_target_no_work = None;
        self.planning_inspection.withdraw();
        if self.original.is_some() {
            return Err(invalid("retain the original stock targets; never recalculate their amounts"));
        }
        let span = request.deadline_tick.checked_sub(c.anchor.tick.get()).filter(|span| {
            *span > 0 && *span <= dfmcp_intent::effects::MAX_DEFAULT_OBLIGATION_TICKS
        }).ok_or_else(|| invalid("stock-target deadline must be future and within one game year"))?;
        if request.poll_interval_ticks.checked_mul(u64::from(request.stable_samples))
            .is_none_or(|duration| duration > span)
        {
            return Err(invalid("stock-target deadline cannot contain its cadence and stability"));
        }
        let basis = self.basis.as_ref()
            .ok_or_else(|| invalid("observe selected workers before stock-target planning"))?;
        if request.observation()? != basis.seal {
            return Err(denied("stock-target portfolio names another original observation"));
        }
        if request.unit_ids()?.iter().any(|id| {
            basis.projection.snapshot.graph.entities.get(id)
                .is_none_or(|unit| unit.kind != dfmcp_world::EntityKind::Unit)
        }) {
            return Err(invalid("stock-target workers are outside the original canonical roster"));
        }
        let mut work = Work::new(c)?;
        let assessments = acquire(
            &self.config.orders, &request.targets, &mut self.observation_tick_floor,
            &work.take(CENSUS_BYTES)?,
        )?;
        let binding = StockTargetsBinding::from_assessments(&assessments)?;
        if assessments.iter().all(|assessment| assessment.amount().is_none()) {
            self.stock_targets_no_work = Some(report(
                &binding, false, self.observation_tick_floor, false, None, None,
            ));
            return self.view(&mut work);
        }
        self.plan_existing_workers(
            request.observation()?, (request.unit_ids()?, request.labor.clone()),
            true, &[], &work.current()?,
            |evidence, workers, _, current, origin| {
                request.compile(&assessments, evidence, workers, current, origin)
            },
        )
    }

    /// Refresh historical receipt custody in this exact operation before the
    /// native guard can accept a nonempty prefix. No prior call's selection is
    /// reused, and this read preserves the separately prepared current order.
    pub(super) fn arm_stock_targets_prefix(&mut self, work: &mut Work) -> Result<()> {
        let Some(targets) = self.stock_targets.clone() else {
            return Ok(());
        };
        targets.withdraw();
        let view = self.frontier.as_mut()
            .ok_or_else(|| invalid("stock-target frontier is absent"))?
            .inspect(&work.take(8 * MIB)?)?;
        let selected = view.entries.iter().find(|entry| entry.phase == Phase::Prepared)
            .map(|entry| entry.step).or(view.next_ready);
        let Some(selected) = selected else {
            return Ok(());
        };
        let plan = self.original.as_ref()
            .ok_or_else(|| invalid("stock-target original is absent"))?;
        let archive = self.seal.ok_or_else(|| invalid("stock-target archive is absent"))?;
        let mut results = Vec::with_capacity(selected.get() as usize);
        for index in 0..selected.get() {
            let step = StepId::new(index);
            let entry = view.entries.iter().find(|entry| entry.step == step)
                .filter(|entry| entry.phase == Phase::Applied)
                .ok_or_else(|| denied("stock-target predecessor is not an original Applied effect"))?;
            let result = self.execution.as_mut()
                .ok_or_else(|| invalid("stock-target executor is absent"))?
                .inspect_retained_order_custody(step, &work.take(PREFIX_CUSTODY_BYTES)?)?;
            let native_receipt = result.native_record().effect().receipt()
                .ok_or_else(|| denied("stock-target predecessor lacks its Created receipt"))?;
            let mut bytes = b"dfmcp-live-production-family-receipt/1\0".to_vec();
            bytes.extend_from_slice(b"work-orders/1.10");
            bytes.extend_from_slice(result.review().seal().as_bytes());
            bytes.extend_from_slice(native_receipt.as_bytes());
            if entry.receipt.is_none_or(|receipt| {
                receipt.family != dfmcp_adapter::live_routing::LiveFamily::WorkOrdersV1_10
                    || receipt.digest != Digest32::of_bytes(&bytes)
            }) {
                return Err(denied("stock-target frontier differs from its actual Created receipt"));
            }
            results.push(result);
        }
        targets.arm_prefix(plan, archive, self.order_journal, selected, &results, &work.current()?)
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
