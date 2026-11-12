//! Original-bound input supply, sharing stock censuses at dispatch boundaries.
//! The archive owns the compact original marker. Fresh reads create historical
//! assessments only; this owner never reserves items or grants mutation authority.
use super::input_binding::InputBinding;
use super::input_config::{InputCheckpoint, InputConfig, InputFloor};
use super::*;
use dfmcp_adapter::live_operations::LiveOperationsObservation;
use dfmcp_adapter::semantic_work_orders::{
    FURNITURE_INPUT_POLICY, FurnitureInputRequirements, INPUT_SUPPLY_QUALIFY_BYTES,
    InputSupplyAssessment, InputSupplySource, OperationsGoalSource, SemanticWorkOrderResult,
};
use dfmcp_adapter::work_orders::{WorkOrderObservation, WorkOrderSpec};
use dfmcp_core::{DfmcpError, ErrorCode, StepId};
use std::sync::{Arc, Mutex};

pub(super) const ASSESS_BYTES: u64 = INPUT_SUPPLY_QUALIFY_BYTES;
// Keep a prepaid local control context for the final authority and queue checks
// after the raw assessment and native read contexts have both been consumed.
pub(super) const GUARD_BYTES: u64 = ASSESS_BYTES + MIB;
pub(super) const CAPTURED_PLAN_BYTES: u64 = GUARD_BYTES + orders::native::STOCK_TARGET_QUEUE_BYTES;
pub(super) const CHECK_BYTES: u64 = 32 * MIB + CAPTURED_PLAN_BYTES;

pub(super) struct PlanInputs {
    pub(super) orders: Vec<WorkOrderSpec>,
    pub(super) captured: bool,
}
impl PlanInputs {
    pub(super) fn requested<'a>(orders: impl IntoIterator<Item = (&'a str, u32)>) -> Result<Self> {
        Ok(Self {
            orders: orders
                .into_iter()
                .map(|(recipe, amount)| {
                    WorkOrderSpec::new(super::request::recipe(recipe)?.0, amount)
                })
                .collect::<Result<Vec<_>>>()?,
            captured: false,
        })
    }
    pub(super) fn captured(
        assessments: &[dfmcp_adapter::semantic_work_orders::StockTargetAssessment],
    ) -> Result<Self> {
        Ok(Self {
            orders: assessments
                .iter()
                .filter_map(|assessment| {
                    assessment
                        .amount()
                        .map(|amount| WorkOrderSpec::new(assessment.recipe(), amount))
                })
                .collect::<Result<Vec<_>>>()?,
            captured: true,
        })
    }
}

struct Owner {
    config: InputConfig,
    shell: O,
    source: InputSupplySource,
    policy_witness: Digest32,
    floor: InputFloor,
    requirements: Option<FurnitureInputRequirements>,
    binding: Option<InputBinding>,
    original_anchor: Option<StateAnchor>,
    original_digest: Option<Digest32>,
    prefix: Option<super::stock_targets::ReceiptPrefix>,
    last: Option<InputSupplyAssessment>,
    revalidation_failed: bool,
}

fn conflict(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::Conflict, message)
}

fn floor_json(floor: InputFloor) -> Value {
    json!({"tick":floor.tick,"generation":floor.generation,"next_job_id":floor.next_job_id,
        "next_building_id":floor.next_building_id,"next_item_id":floor.next_item_id,"fenced":floor.fenced})
}

impl Owner {
    fn current(&self, c: &OperationContext) -> Result<()> {
        self.shell.check_environment()?;
        inventory_baseline::authorize_observe(c, GameTick(self.floor.tick))?;
        if self.floor.fenced {
            return Err(denied("input supply source continuity is fenced"));
        }
        Ok(())
    }

    fn note(&mut self, operations: &LiveOperationsObservation) -> Result<()> {
        self.last = None;
        operations.validate()?;
        if !self.source.matches_capture_identity(operations) {
            self.floor.fenced = true;
            // Only a matching fortress/software can advance its incarnation.
            let pins = self.shell.acceptance();
            if operations.jobs.world_folder == pins.world_folder
                && operations.jobs.site_id == pins.site_id as i32
                && operations.jobs.df_version == pins.df_version
                && operations.jobs.dfhack_version == pins.dfhack_version
            {
                self.floor.generation =
                    self.floor.generation.max(operations.jobs.bridge_generation);
            }
            return Err(denied(
                "input supply source differs from its exact operator pins",
            ));
        }
        let tick = operations.jobs.tick().get();
        let counters = [
            operations.jobs.next_job_id,
            operations.next_building_id,
            operations.next_item_id,
        ];
        if tick < self.floor.tick
            || counters
                .into_iter()
                .zip(self.floor.counters())
                .any(|(new, old)| new < old)
        {
            self.floor.fenced = true;
        }
        self.floor.tick = self.floor.tick.max(tick);
        self.floor.next_job_id = self.floor.next_job_id.max(counters[0]);
        self.floor.next_building_id = self.floor.next_building_id.max(counters[1]);
        self.floor.next_item_id = self.floor.next_item_id.max(counters[2]);
        if self.floor.fenced {
            return Err(denied("input supply source tick or allocator regressed"));
        }
        Ok(())
    }

    fn assess(
        &mut self,
        operations: &LiveOperationsObservation,
        c: &OperationContext,
    ) -> Result<InputSupplyAssessment> {
        self.note(operations)?;
        self.current(c)?;
        let original = self
            .original_anchor
            .ok_or_else(|| invalid("input supply original anchor is absent"))?;
        let mut current = c.clone();
        current.anchor = original;
        let requirements = self
            .requirements
            .as_ref()
            .ok_or_else(|| invalid("input supply complete demand is absent"))?;
        let assessment = InputSupplyAssessment::qualify(
            requirements,
            &self.source,
            original,
            operations,
            &current,
        )?;
        self.current(c)?;
        self.last = Some(assessment.clone());
        if !assessment.ready() {
            return Err(conflict(
                "input supply shortage: complete original orders and retained WOOD reserve cannot be funded together",
            ));
        }
        Ok(assessment)
    }

    fn queue_scope(&mut self, queue: &WorkOrderObservation, c: &OperationContext) -> Result<()> {
        self.floor.tick = self.floor.tick.max(queue.tick());
        self.current(c)?;
        let original = self
            .original_anchor
            .ok_or_else(|| invalid("input supply original anchor is absent"))?;
        let pins = self.shell.acceptance();
        if c.anchor.fortress_id != original.fortress_id
            || c.anchor.cursor.epoch != original.cursor.epoch
            || c.anchor.tick != original.tick
            || queue.fortress_id() != original.fortress_id
            || queue.world_folder() != pins.world_folder
            || queue.site_id() != pins.site_id
            || queue.generation() != pins.work_order_generation
            || !queue.paused()
            || queue.tick() != original.tick.get()
        {
            return Err(denied(
                "input supply requires its original paused native queue and source tick",
            ));
        }
        Ok(())
    }

    fn read_queue(&mut self, c: &OperationContext) -> Result<WorkOrderObservation> {
        let mut tick = GameTick(self.floor.tick);
        let result = self.shell.stock_target_queue(c, &mut tick);
        self.floor.tick = self.floor.tick.max(tick.get());
        result
    }

    fn initial(
        &mut self,
        inputs: &[WorkOrderSpec],
        captured: Option<&LiveOperationsObservation>,
        c: &OperationContext,
    ) -> Result<InputBinding> {
        self.last = None;
        self.prefix = None;
        if self.original_digest.is_some() {
            return Err(invalid("input supply cannot replace an original demand"));
        }
        self.binding = None;
        self.current(c)?;
        self.requirements = Some(FurnitureInputRequirements::new(
            inputs,
            self.config.reserve_wood_stack_units,
        )?);
        self.original_anchor = Some(c.anchor);
        let mut work = Work::new(c)?;
        let assessment = if let Some(capture) = captured {
            self.assess(capture, &work.take(ASSESS_BYTES)?)?
        } else {
            let mut source = self
                .shell
                .operations(&work.take(16 * MIB)?)
                .inspect_err(|_| {
                    self.floor.fenced = true;
                })?;
            let read = work.take(16 * MIB).inspect_err(|_| {
                self.floor.fenced = true;
            })?;
            let capture = source.read_operations(&read).inspect_err(|_| {
                self.floor.fenced = true;
            })?;
            self.note(&capture)?;
            drop(source);
            self.assess(&capture, &work.take(ASSESS_BYTES)?)?
        };
        let queue = self.read_queue(&work.take(orders::native::STOCK_TARGET_QUEUE_BYTES)?)?;
        self.queue_scope(&queue, &work.current()?)?;
        if !queue.order_ids().is_empty() {
            return Err(conflict(
                "input supply cannot credit unclassified competing manager orders",
            ));
        }
        let binding = InputBinding::new(&assessment)?;
        self.binding = Some(binding);
        self.current(&work.current()?)?;
        Ok(binding)
    }

    fn guarded_queue(&mut self, queue: &WorkOrderObservation, c: &OperationContext) -> Result<()> {
        self.queue_scope(queue, c)?;
        if self.original_digest.is_none()
            || self
                .prefix
                .as_ref()
                .is_none_or(|prefix| !prefix.matches(queue, c))
        {
            return Err(conflict(
                "input supply requires the exact receipt-backed original order prefix",
            ));
        }
        Ok(())
    }

    fn restore(&mut self, floor: InputFloor, c: &OperationContext) -> Result<()> {
        floor.validate(self.source.generation(), floor.tick)?;
        if floor.tick < self.floor.tick
            || floor.generation < self.floor.generation
            || floor
                .counters()
                .into_iter()
                .zip(self.floor.counters())
                .any(|(new, old)| new < old)
            || (self.floor.fenced && !floor.fenced)
        {
            return Err(denied(
                "input supply recovery cannot erase consumed source floors",
            ));
        }
        let mut current = c.clone();
        current.anchor.tick = current.anchor.tick.max(GameTick(floor.tick));
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        self.floor = floor;
        self.last = None;
        self.prefix = None;
        Ok(())
    }

    fn origin_witness(&self) -> Digest32 {
        if self.original_digest.is_some() {
            self.binding
                .map_or(self.policy_witness, InputBinding::witness)
        } else {
            self.policy_witness
        }
    }

    fn status(&self) -> Value {
        let mut checkpoint = floor_json(self.floor);
        checkpoint["origin_witness"] = json!(self.origin_witness().to_hex());
        json!({"schema":"dfmcp.furniture-input-supply/1","policy":FURNITURE_INPUT_POLICY,
            "status":if self.floor.fenced {"fenced"} else if self.last.as_ref().is_some_and(|v|!v.ready()) {"shortage"} else if self.original_digest.is_some() {"planned"} else {"configured"},
            "scope":"original_complete_input_dispatch_prerequisite",
            "origin_witness":self.origin_witness().to_hex(),
            "binding_digest":self.original_digest.and(self.binding).map(|b|b.witness().to_hex()),
            "source_generation":self.source.generation(),"required_stack_units":self.requirements.as_ref().map(FurnitureInputRequirements::required_stack_units),
            "reserve_wood_stack_units":self.config.reserve_wood_stack_units,
            "orders":self.requirements.as_ref().map(|r|r.orders().iter().map(|o|json!({"recipe":o.recipe().as_str(),"amount":o.amount()})).collect::<Vec<_>>()).unwrap_or_default(),
            "consumed_tick_floor":self.floor.tick,"source_fenced":self.floor.fenced,"revalidation_failed":self.revalidation_failed,
            "checkpoint":checkpoint,
            "last_qualified":self.last.as_ref().map(|a|json!({"tick":a.original_anchor().tick.get(),"witness":a.witness().to_hex(),
                "canonical_parent":anchor_json(a.original_anchor()),"status":if a.ready(){"ready"}else{"shortage"},
                "eligible_stack_units":a.eligible_stack_units(),"required_stack_units":a.required_stack_units(),
                "shortfall_stack_units":a.shortfall_stack_units(),"excluded_items":a.excluded_items(),
                "source_digest":a.source_digest().to_hex(),"fresh_for_current_call":false})),
            "current_state_proven":false,"production_feasibility_proven":false,"reservation_created":false,"mutation_authority":false})
    }
}

#[derive(Clone)]
pub(super) struct SharedInputSupply(Arc<Mutex<Owner>>);

impl SharedInputSupply {
    pub(super) fn new(config: InputConfig, shell: O) -> Result<Self> {
        let pins = shell.acceptance();
        let source = InputSupplySource::new(
            pins.world_folder.clone(),
            pins.site_id,
            pins.df_version.clone(),
            pins.dfhack_version.clone(),
            pins.operations_generation,
        )?;
        let mut bytes = b"dfmcp-combined-input-supply-policy/1\0".to_vec();
        bytes.extend_from_slice(source.witness().as_bytes());
        bytes.extend_from_slice(&config.reserve_wood_stack_units.to_be_bytes());
        bytes.extend_from_slice(FURNITURE_INPUT_POLICY.as_bytes());
        let floor = InputFloor {
            tick: 0,
            generation: source.generation(),
            next_job_id: 0,
            next_building_id: 0,
            next_item_id: 0,
            fenced: false,
        };
        Ok(Self(Arc::new(Mutex::new(Owner {
            config,
            shell,
            source,
            policy_witness: Digest32::of_bytes(&bytes),
            floor,
            requirements: None,
            binding: None,
            original_anchor: None,
            original_digest: None,
            prefix: None,
            last: None,
            revalidation_failed: false,
        }))))
    }
    pub(super) fn high_tick(&self) -> GameTick {
        self.0
            .try_lock()
            .map_or(GameTick(u64::MAX), |owner| GameTick(owner.floor.tick))
    }
    pub(super) fn withdraw(&self) {
        if let Ok(mut owner) = self.0.try_lock() {
            if owner
                .last
                .as_ref()
                .is_some_and(InputSupplyAssessment::ready)
            {
                owner.last = None;
            }
            owner.prefix = None;
        }
    }
    pub(super) fn native_failed(&self) {
        if let Ok(mut owner) = self.0.try_lock() {
            owner.floor.fenced = true;
            owner.revalidation_failed = true;
            owner.last = None;
        }
    }
    pub(super) fn guard_failed(&self) {
        if let Ok(mut owner) = self.0.try_lock() {
            owner.revalidation_failed = true;
            if owner
                .last
                .as_ref()
                .is_some_and(InputSupplyAssessment::ready)
            {
                owner.last = None;
            }
        }
    }
    pub(super) fn note_capture(&self, capture: &LiveOperationsObservation) -> Result<()> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("input supply owner is unavailable"))?;
        let result = owner.note(capture);
        owner.revalidation_failed = result.is_err();
        result
    }
    pub(super) fn prepare(
        &self,
        orders: &[WorkOrderSpec],
        capture: Option<&LiveOperationsObservation>,
        c: &OperationContext,
    ) -> Result<InputBinding> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("input supply owner is unavailable"))?;
        let result = owner.initial(orders, capture, c);
        owner.revalidation_failed = result.is_err();
        result
    }
    pub(super) fn prepared(
        &self,
        inputs: &[WorkOrderSpec],
        c: &OperationContext,
    ) -> Result<InputBinding> {
        let owner = self
            .0
            .try_lock()
            .map_err(|_| denied("input supply owner is unavailable"))?;
        owner.current(c)?;
        let requirements =
            FurnitureInputRequirements::new(inputs, owner.config.reserve_wood_stack_units)?;
        let binding = owner
            .binding
            .ok_or_else(|| denied("input supply has no complete original assessment"))?;
        if owner.original_digest.is_some()
            || owner.original_anchor != Some(c.anchor)
            || owner.requirements.as_ref() != Some(&requirements)
            || owner.last.as_ref().is_none_or(|a| !a.ready())
        {
            return Err(denied(
                "input supply preplanning assessment does not match complete original demand",
            ));
        }
        Ok(binding)
    }
    pub(super) fn bind(&self, plan: &PreparedPlan, c: &OperationContext) -> Result<()> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("input supply owner is unavailable"))?;
        owner.current(c)?;
        let binding = owner
            .binding
            .ok_or_else(|| denied("input supply original assessment is absent"))?;
        if prerequisite_binding::requirements(plan)?.input_supply != Some(binding)
            || owner.original_digest.is_some()
            || owner.original_anchor != Some(plan.anchor)
            || owner
                .last
                .as_ref()
                .is_none_or(|a| !a.ready() || a.witness() != binding.assessment)
        {
            return Err(denied(
                "input supply marker lost its exact preplanning assessment",
            ));
        }
        owner.requirements = Some(binding.validate_original(plan, owner.config, &owner.source)?);
        owner.original_digest = Some(plan.digest);
        Ok(())
    }
    pub(super) fn reopen(
        &self,
        plan: &PreparedPlan,
        digest: Digest32,
        floor: InputFloor,
        c: &OperationContext,
    ) -> Result<()> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("input supply owner is unavailable"))?;
        let binding = prerequisite_binding::requirements(plan)?
            .input_supply
            .ok_or_else(|| denied("original input supply marker is missing"))?;
        if binding.witness() != digest || owner.original_digest.is_some() {
            return Err(denied(
                "input supply recovery changed its original marker commitment",
            ));
        }
        owner.requirements = Some(binding.validate_original(plan, owner.config, &owner.source)?);
        owner.binding = Some(binding);
        owner.original_anchor = Some(plan.anchor);
        owner.original_digest = Some(plan.digest);
        owner.floor = binding.floor();
        owner.restore(floor, c)
    }
    pub(super) fn restore_checkpoint(
        &self,
        checkpoint: InputCheckpoint,
        c: &OperationContext,
    ) -> Result<()> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("input supply owner is unavailable"))?;
        if owner.original_digest.is_some() || checkpoint.origin_witness != owner.policy_witness {
            return Err(denied(
                "input supply checkpoint changed its source or reserve policy",
            ));
        }
        owner.restore(checkpoint.floor, c)
    }
    pub(super) fn arm_prefix(
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
            .map_err(|_| denied("input supply owner is unavailable"))?;
        owner.prefix = None;
        owner.last = None;
        owner.current(c)?;
        if owner.original_digest != Some(plan.digest) || owner.original_anchor != Some(plan.anchor)
        {
            return Err(denied(
                "input supply queue prefix belongs to another original",
            ));
        }
        // This derives only historical Created membership, not remaining input
        // credit: every original order stays in the conserved requirement.
        owner.prefix = Some(super::stock_targets::verified_prefix(
            plan, archive, journal, step, results, c,
        )?);
        Ok(())
    }
    pub(super) fn begin_guard(
        &self,
        queue: &WorkOrderObservation,
        c: &OperationContext,
    ) -> Result<()> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("input supply owner is unavailable"))?;
        owner.last = None;
        owner.revalidation_failed = true;
        owner.guarded_queue(queue, c)
    }
    pub(super) fn qualify_captured(
        &self,
        capture: &LiveOperationsObservation,
        c: &OperationContext,
    ) -> Result<()> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("input supply owner is unavailable"))?;
        let result = owner.assess(capture, c).map(|_| ());
        owner.revalidation_failed = result.is_err();
        result
    }
    pub(super) fn finish_guard(
        &self,
        queue: &WorkOrderObservation,
        c: &OperationContext,
    ) -> Result<()> {
        let mut owner = self
            .0
            .try_lock()
            .map_err(|_| denied("input supply owner is unavailable"))?;
        let result =
            owner.guarded_queue(queue, c).and_then(|()| {
                if owner.last.as_ref().is_some_and(|a| {
                    a.ready() && Some(a.original_anchor()) == owner.original_anchor
                }) {
                    Ok(())
                } else {
                    Err(denied(
                        "input supply has no fresh complete dispatch assessment",
                    ))
                }
            });
        owner.revalidation_failed = result.is_err();
        result
    }
    pub(super) fn qualify(&self, queue: &WorkOrderObservation, c: &OperationContext) -> Result<()> {
        let result = self.qualify_inner(queue, c);
        if result.is_err() {
            self.guard_failed();
        }
        result
    }
    fn qualify_inner(&self, queue: &WorkOrderObservation, c: &OperationContext) -> Result<()> {
        self.begin_guard(queue, c)?;
        let mut work = Work::new(c)?;
        if work.bytes < CHECK_BYTES {
            return Err(exhausted(
                "input supply census and queue checks must be prepaid",
            ));
        }
        let shell = self
            .0
            .try_lock()
            .map_err(|_| denied("input supply owner is unavailable"))?
            .shell
            .clone();
        let mut source = shell
            .operations(&work.take(16 * MIB)?)
            .inspect_err(|_| self.native_failed())?;
        let read = work.take(16 * MIB).inspect_err(|_| self.native_failed())?;
        let capture = source
            .read_operations(&read)
            .inspect_err(|_| self.native_failed())?;
        self.note_capture(&capture)?;
        drop(source);
        self.qualify_captured(&capture, &work.take(ASSESS_BYTES)?)?;
        let fresh = {
            let mut owner = self
                .0
                .try_lock()
                .map_err(|_| denied("input supply owner is unavailable"))?;
            owner.read_queue(&work.take(orders::native::STOCK_TARGET_QUEUE_BYTES)?)?
        };
        if fresh != *queue {
            return Err(denied("input supply queue changed after fresh census"));
        }
        self.finish_guard(&fresh, &work.current()?)
    }
    pub(super) fn status(&self) -> Value {
        self.0.try_lock().map_or_else(
            |_| json!({"status":"unavailable","current_state_proven":false}),
            |owner| owner.status(),
        )
    }
    pub(super) fn seal(&self) -> Option<Digest32> {
        self.0.try_lock().ok().and_then(|owner| {
            owner
                .original_digest
                .and(owner.binding)
                .map(InputBinding::witness)
        })
    }
    pub(super) fn recovery_floor(&self) -> Value {
        self.0
            .try_lock()
            .map_or(Value::Null, |owner| floor_json(owner.floor))
    }
}
