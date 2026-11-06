//! Explicit bounded time for the original selected production objective.
//!
//! The existing native owner supplies the durable marker, sole unpause attempt
//! and recovery semantics. This parent retains the complete original archive
//! and predecessor receipts at every dispatch edge. Native clock receipts are
//! historical time-control evidence; current parent proof still requires fresh
//! joined labor and inventory observations.
use super::*;
use dfmcp_adapter::live_production::StepEvidence;
use dfmcp_adapter::order_run::journal::{OrderRunJournal, OrderRunState};
use dfmcp_adapter::order_run::private_file::PrivateOrderRunFile;
use dfmcp_adapter::semantic_work_orders::{ObservedInventoryObjective, SemanticWorkOrderResult};
use dfmcp_adapter::work_order_control::CreationState;
use dfmcp_intent::ObligationStatus;
use orders::clock::{Clock, ClockTarget, ClockView, RunRequest};

/// Retained target identity is immutable historical custody, never a dispatch
/// permit. Only a fresh source sample plus the guarded original custody check
/// may call the existing native clock owner's effect methods.
pub(super) struct ProductionClock {
    inner: Clock<PrivateOrderRunFile>,
    target: Option<ClockTarget>,
    latest: Option<ClockView>,
    inspection_required: bool,
}
impl ProductionClock {
    pub(super) fn new(journal: OrderRunJournal<PrivateOrderRunFile>) -> Self {
        Self {
            inner: Clock::new(journal),
            target: None,
            latest: None,
            inspection_required: false,
        }
    }
    fn bind(
        &mut self,
        original: &PreparedPlan,
        archive: PlanArchiveSeal,
        journal: Digest32,
        result: &SemanticWorkOrderResult,
    ) -> Result<()> {
        if ObservedInventoryObjective::from_selected_plan(original, result.review().step_id())
            .is_none()
        {
            return Err(denied(
                "combined clock requires the original explicit inventory objective",
            ));
        }
        let target = ClockTarget::for_selected_original(original, archive, journal, result)?;
        self.bind_target(target)
    }
    fn bind_target(&mut self, target: ClockTarget) -> Result<()> {
        if self
            .target
            .as_ref()
            .is_some_and(|retained| retained.digest() != target.digest())
        {
            return Err(denied(
                "combined clock cannot replace its original selected target",
            ));
        }
        self.target = Some(target);
        Ok(())
    }
    fn inspect(&mut self, c: &OperationContext) -> Result<ClockView> {
        self.inspection_required = true;
        let view = self.inner.inspect(self.target.as_ref(), c)?;
        self.latest = Some(view.clone());
        self.inspection_required = false;
        Ok(view)
    }
    pub(super) fn high_tick(&self) -> u64 {
        self.inner
            .observed_tick_floor()
            .max(self.latest.as_ref().map_or(0, |view| view.high_tick))
    }
    pub(super) fn journal_id(&self) -> Option<Digest32> {
        self.latest.as_ref().map(|view| view.journal_id)
    }
    pub(super) fn may_be_running(&self) -> bool {
        self.inspection_required || self.latest.as_ref().is_some_and(ClockView::may_be_running)
    }
    pub(super) fn pending(&self) -> bool {
        self.inspection_required
            || self
                .latest
                .as_ref()
                .is_some_and(|view| view.pending || view.unresolved)
    }
    pub(super) fn has_runs(&self) -> bool {
        self.latest.as_ref().is_some_and(ClockView::has_runs)
    }
    pub(super) fn historical_advancement(&self) -> Value {
        let Some(view) = &self.latest else {
            return Value::Null;
        };
        if view.runs.iter().any(|run| {
            run.entry
                .native()
                .and_then(|native| native.observed_tick())
                .is_some_and(|tick| tick > run.entry.plan().before().tick())
        }) {
            return json!(true);
        }
        if self.inspection_required
            || view.unresolved
            || view.runs.iter().any(|run| {
                run.entry.native().is_some_and(|native| {
                    native.unpause_attempted() && native.observed_tick().is_none()
                })
            })
        {
            return Value::Null;
        }
        json!(false)
    }
    pub(super) fn to_json(&self) -> Value {
        self.latest.as_ref().map_or(Value::Null, |view| {
            let mut value = view.to_json();
            value["source"] = json!("order-run/1.14");
            value["target_scope"] = json!("selected_original_order_step");
            value["independent_authority"] = json!(true);
            value["original_deadline_renewed"] = json!(false);
            value["automatic_dispatch"] = json!(false);
            value["inspection_required"] = json!(self.inspection_required);
            if self.inspection_required {
                // A failed publication cannot relabel the last cached snapshot
                // as a fresh clean journal, even when the native reply was lost.
                value["pending"] = json!(true);
                value["unresolved"] = json!(true);
                value["may_be_running"] = json!(true);
                value["needs_reconciliation"] = json!(true);
            }
            value
        })
    }
}

const TARGET_BYTES: u64 = 1024 * MIB;
const GUARDED_TARGET_BYTES: u64 = 1600 * MIB;
const CLOCK_VIEW_BYTES: u64 = 8 * MIB;
const CLOCK_ACTION_BYTES: u64 = 96 * MIB;

impl Runtime {
    pub(crate) fn clock_control_enabled(&self) -> bool {
        self.opening_clock
            && self.mode == Mode::Control
            && self.config.orders.clock_enabled()
            && self
                .clock
                .as_ref()
                .is_some_and(|clock| clock.inner.is_control())
    }
    pub(super) fn clock_json(&self) -> Value {
        self.clock.as_ref().map_or(Value::Null, |clock| {
            let mut value = clock.to_json();
            if value.is_object() {
                value["review_available"] = json!(
                    !clock.inspection_required
                        && self.clock_review.is_some_and(|seal| {
                            clock
                                .latest
                                .as_ref()
                                .and_then(ClockView::active)
                                .is_some_and(|run| {
                                    run.review_seal == seal
                                        && run.entry.state() == OrderRunState::Prepared
                                        && run.entry.plan().before().tick()
                                            == self.anchor.tick.get()
                                })
                        })
                );
            }
            value
        })
    }
    pub(super) fn clock_advancement_json(&self) -> Value {
        let order = self
            .clock
            .as_ref()
            .map_or(json!(false), ProductionClock::historical_advancement);
        let inventory = self.inventory_clock.as_ref().map_or(
            json!(false),
            ProductionInventoryClock::historical_advancement,
        );
        if order == true || inventory == true {
            json!(true)
        } else if order.is_null() || inventory.is_null() {
            Value::Null
        } else {
            json!(false)
        }
    }
    pub(super) fn raise_clock_floor(&mut self) {
        if let Some(clock) = &self.clock {
            self.observation_tick_floor =
                self.observation_tick_floor.max(GameTick(clock.high_tick()));
        }
        if let Some(clock) = &self.inventory_clock {
            self.observation_tick_floor =
                self.observation_tick_floor.max(GameTick(clock.high_tick()));
        }
        if let Some(execution) = &self.execution {
            self.observation_tick_floor = self
                .observation_tick_floor
                .max(GameTick(execution.high_tick()));
        }
    }
    pub(super) fn clock_withholds_goal(&self) -> bool {
        self.clock_invalidates_goal
            || self.inventory_clock_invalidates_goal
            || self
                .clock
                .as_ref()
                .is_some_and(ProductionClock::may_be_running)
            || self
                .inventory_clock
                .as_ref()
                .is_some_and(ProductionInventoryClock::may_be_running)
    }
    /// A newly settled executor result has already passed complete predecessor
    /// and final custody checks. Retain only its immutable clock identity here;
    /// no preparation permit crosses into the clock owner.
    pub(super) fn bind_created_clock_target(&mut self, settled: bool) -> Result<()> {
        if !settled
            || self
                .clock
                .as_ref()
                .is_none_or(|clock| clock.target.is_some())
        {
            return Ok(());
        }
        let Some(StepEvidence::WorkOrder(result)) = self
            .execution
            .as_ref()
            .and_then(Execution::latest_step_evidence)
        else {
            return Ok(());
        };
        if result.native_record().state() != CreationState::Created {
            return Ok(());
        }
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| invalid("original plan absent"))?;
        if ObservedInventoryObjective::from_selected_plan(original, result.review().step_id())
            .is_none()
        {
            return Ok(());
        }
        self.clock
            .as_mut()
            .ok_or_else(|| invalid("clock owner absent"))?
            .bind(
                original,
                self.seal
                    .ok_or_else(|| invalid("original archive absent"))?,
                self.order_journal,
                result,
            )
    }
    /// Recovered target identity is read from the original private child
    /// custodians. Query alone suffices; no source or preparation is restored.
    pub(super) fn restore_clock_target(&mut self, work: &mut Work) -> Result<()> {
        if self
            .clock
            .as_ref()
            .is_none_or(|clock| clock.target.is_some())
        {
            return Ok(());
        }
        let Some(original) = &self.original else {
            return Ok(());
        };
        if !original
            .steps
            .iter()
            .any(|step| ObservedInventoryObjective::from_selected_plan(original, step.id).is_some())
        {
            return Ok(());
        }
        let ready = self
            .frontier
            .as_mut()
            .ok_or_else(|| denied("original frontier absent"))?
            .inspect(&work.take(CLOCK_VIEW_BYTES)?)?;
        if !ready.all_applied() || ready.needs_reconciliation() {
            return Ok(());
        }
        self.bind_created_clock_target(true)?;
        if self
            .clock
            .as_ref()
            .is_some_and(|clock| clock.target.is_none())
        {
            let target = self.inspect_original_clock_target(&work.take(GUARDED_TARGET_BYTES)?)?;
            self.clock
                .as_mut()
                .ok_or_else(|| denied("clock owner absent"))?
                .bind_target(target)?;
        }
        Ok(())
    }
    fn inspect_original_clock_target(&mut self, c: &OperationContext) -> Result<ClockTarget> {
        self.inspect_original_goal_clock_target(c, ClockTarget::for_selected_original)
    }
    /// Both clock families retain the complete original archive and predecessor
    /// custody. The target builder only chooses the domain-separated clock key.
    pub(super) fn inspect_original_goal_clock_target<T>(
        &mut self,
        c: &OperationContext,
        build: impl FnOnce(
            &PreparedPlan,
            PlanArchiveSeal,
            Digest32,
            &SemanticWorkOrderResult,
        ) -> Result<T>,
    ) -> Result<T> {
        self.config.current()?;
        let mut work = Work::new(c)?;
        self.arm(&mut work)?;
        let custody = self
            .custody
            .as_ref()
            .ok_or_else(|| denied("original custody absent"))?
            .clone();
        let result = (|| {
            custody.verify(&work.current()?)?;
            let frontier = self
                .frontier
                .as_mut()
                .ok_or_else(|| denied("original frontier absent"))?
                .inspect(&work.take(CLOCK_VIEW_BYTES)?)?;
            if !frontier.all_applied() || frontier.needs_reconciliation() {
                return Err(denied("clock requires both original effects to be settled"));
            }
            let original = self
                .original
                .as_ref()
                .ok_or_else(|| denied("original plan absent"))?;
            let step = original
                .steps
                .iter()
                .find(|step| {
                    ObservedInventoryObjective::from_selected_plan(original, step.id).is_some()
                })
                .map(|step| step.id)
                .ok_or_else(|| {
                    denied("clock requires the original explicit inventory objective")
                })?;
            let result = self
                .execution
                .as_mut()
                .ok_or_else(|| denied("original executor absent"))?
                .inspect_selected_order_custody(step, &work.take(TARGET_BYTES)?)?;
            let target = build(
                original,
                self.seal.ok_or_else(|| denied("original archive absent"))?,
                self.order_journal,
                &result,
            )?;
            custody.verify(&work.current()?)?;
            self.config.current()?;
            work.current()?;
            Ok(target)
        })();
        let disarmed = custody.disarm();
        self.raise_clock_floor();
        let target = result?;
        disarmed?;
        Ok(target)
    }
    pub(super) fn refresh_clock_view(&mut self, c: &OperationContext) -> Result<()> {
        let result = self
            .clock
            .as_mut()
            .map(|clock| clock.inspect(c))
            .transpose();
        self.raise_clock_floor();
        result.map(|_| ())
    }
    /// Wait queries only the already retained run. It never prepares, commits,
    /// retries an unpause or manufactures a current paused game observation.
    pub(super) fn reconcile_clock(&mut self, work: &mut Work) -> Result<()> {
        if self.clock.is_none() {
            return Ok(());
        }
        self.restore_clock_target(work)?;
        self.refresh_clock_view(&work.take(CLOCK_VIEW_BYTES)?)?;
        let needed = self
            .clock
            .as_ref()
            .and_then(|clock| clock.latest.as_ref())
            .is_some_and(ClockView::needs_reconciliation);
        if !needed {
            return Ok(());
        }
        let source_context = work.take(4 * MIB)?;
        let run_context = work.take(CLOCK_ACTION_BYTES)?;
        let error_view = work.take(CLOCK_VIEW_BYTES)?;
        let mut source = self.config.orders.clock_source(&source_context)?;
        let clock = self
            .clock
            .as_mut()
            .ok_or_else(|| denied("clock owner absent"))?;
        let target = clock
            .target
            .as_ref()
            .ok_or_else(|| denied("original clock target absent"))?;
        clock.inspection_required = true;
        let result = clock
            .inner
            .reconcile(target, &mut source, &work.narrow(&run_context)?);
        if let Ok(view) = &result {
            clock.latest = Some(view.clone());
            clock.inspection_required = false;
        } else if let Ok(current) = work.narrow(&error_view) {
            let _ = clock.inspect(&current);
        }
        self.raise_clock_floor();
        result.map(|_| ())
    }
    fn check_clock_control(&self, c: &OperationContext, planning: bool) -> Result<()> {
        self.check(c, planning)?;
        if !self.clock_control_enabled() {
            return Err(denied(
                "this combined owner has no current independent clock authority",
            ));
        }
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
    }
    fn sample_clock_target(&mut self, c: &OperationContext) -> Result<ClockTarget> {
        self.wait_with_goal(c)?;
        if self
            .inventory_clock
            .as_ref()
            .is_some_and(ProductionInventoryClock::pending)
        {
            return Err(denied(
                "settle or cancel the retained inventory clock before an order run",
            ));
        }
        let clock = self
            .clock
            .as_ref()
            .ok_or_else(|| denied("clock owner absent"))?;
        if clock
            .latest
            .as_ref()
            .is_some_and(|view| !view.can_sample_goal())
        {
            return Err(denied(
                "settle or cancel the retained clock before another run",
            ));
        }
        let monitor = self
            .goal_monitor
            .as_ref()
            .ok_or_else(|| denied("clock requires a fresh original inventory observation"))?;
        let sample = monitor
            .latest()
            .filter(|sample| {
                sample.anchor.tick == self.anchor.tick
                    && sample.anchor.cursor == self.anchor.cursor
                    && sample.receipt_linked
            })
            .ok_or_else(|| {
                denied("clock requires a fresh receipt-linked original inventory sample")
            })?;
        if self.parent_goal.as_ref().is_none_or(|parent| {
            parent.anchor != self.anchor || parent.selected_anchor != sample.anchor
        }) || self.clock_withholds_goal()
        {
            return Err(denied(
                "clock requires a fresh joined original labor and inventory sample",
            ));
        }
        if sample.current_goal_proven() {
            return Err(denied("the original inventory objective is already proved"));
        }
        if matches!(
            monitor.historical_obligation_status(),
            Some(ObligationStatus::Failed { .. } | ObligationStatus::Cancelled { .. })
        ) {
            return Err(denied("the original inventory obligation is terminal"));
        }
        clock
            .target
            .clone()
            .ok_or_else(|| denied("clock requires exact original Created custody"))
    }
    fn verify_clock_edge(&mut self, target: &ClockTarget, c: &OperationContext) -> Result<()> {
        self.config.current()?;
        if !self.opening_clock || !self.config.orders.clock_enabled() {
            return Err(denied("independent operator clock authority was revoked"));
        }
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        let mut work = Work::new(c)?;
        if self.inventory_clock.is_some() {
            self.refresh_inventory_clock_view(&work.take(CLOCK_VIEW_BYTES)?)?;
        }
        if self
            .inventory_clock
            .as_ref()
            .is_some_and(ProductionInventoryClock::pending)
        {
            return Err(denied(
                "settle or cancel the retained inventory clock before an order run",
            ));
        }
        let current = self.inspect_original_clock_target(&work.rest(GUARDED_TARGET_BYTES)?)?;
        if current.digest() != target.digest() {
            return Err(denied(
                "selected original clock custody changed before its native effect",
            ));
        }
        Ok(())
    }
    pub(crate) fn plan_run(&mut self, request: RunRequest, c: &OperationContext) -> Result<View> {
        self.run_clock(
            c,
            |clock, target, source, current, guard| {
                clock.prepare_guarded(target, &request, source, current, guard)
            },
            false,
        )
    }
    pub(crate) fn commit_run(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
    ) -> Result<View> {
        if !confirmed {
            self.clock_review = None;
            return Err(denied("confirm the exact original-bound clock review"));
        }
        if self.clock_review != Some(seal) {
            self.clock_review = None;
            return Err(denied(
                "prepare and confirm this owner's current exact clock review",
            ));
        }
        self.clock_review = None;
        self.run_clock(
            c,
            |clock, target, source, current, guard| {
                clock.commit_guarded(target, seal, true, source, current, guard)
            },
            true,
        )
    }
    fn run_clock(
        &mut self,
        c: &OperationContext,
        action: impl FnOnce(
            &mut Clock<PrivateOrderRunFile>,
            &ClockTarget,
            &mut orders::native::PinnedClock,
            &OperationContext,
            &mut dyn FnMut(&OperationContext) -> Result<()>,
        ) -> Result<ClockView>,
        advancing: bool,
    ) -> Result<View> {
        let result = self.run_clock_inner(c, action, advancing);
        self.raise_clock_floor();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    fn run_clock_inner(
        &mut self,
        c: &OperationContext,
        action: impl FnOnce(
            &mut Clock<PrivateOrderRunFile>,
            &ClockTarget,
            &mut orders::native::PinnedClock,
            &OperationContext,
            &mut dyn FnMut(&OperationContext) -> Result<()>,
        ) -> Result<ClockView>,
        advancing: bool,
    ) -> Result<View> {
        self.check_clock_control(c, true)?;
        let mut work = Work::new(c)?;
        let output = work.take(256 * MIB)?;
        let source_context = work.take(4 * MIB)?;
        let mut guard_context = Some(work.take(GUARDED_TARGET_BYTES)?);
        let run_context = work.take(CLOCK_ACTION_BYTES)?;
        let error_view = work.take(CLOCK_VIEW_BYTES)?;
        let target = self.sample_clock_target(&work.rest(MAX_BYTES)?)?;
        work.context.anchor = self.anchor;
        let mut source = self
            .config
            .orders
            .clock_source(&work.narrow(&source_context)?)?;
        if advancing {
            self.clock_invalidates_goal = true;
            self.parent_goal = None;
            if let Some(basis) = &mut self.basis {
                basis.choices = None;
            }
        }
        let run_context = work.narrow(&run_context)?;
        let mut clock = self
            .clock
            .take()
            .ok_or_else(|| denied("clock owner absent"))?;
        clock.inspection_required = true;
        let result = action(
            &mut clock.inner,
            &target,
            &mut source,
            &run_context,
            &mut |edge| {
                let mut guarded = guard_context
                    .take()
                    .ok_or_else(|| denied("original clock custody guard cannot repeat"))?;
                guarded.anchor = edge.anchor;
                guarded.budget.max_wall_millis = guarded
                    .budget
                    .max_wall_millis
                    .min(edge.budget.max_wall_millis);
                self.verify_clock_edge(&target, &guarded)
            },
        );
        if let Ok(view) = &result {
            clock.latest = Some(view.clone());
            clock.inspection_required = false;
            if !advancing {
                self.clock_review = view
                    .active()
                    .filter(|run| run.entry.state() == OrderRunState::Prepared)
                    .map(|run| run.review_seal);
            }
        } else if let Ok(current) = work.narrow(&error_view) {
            let _ = clock.inspect(&current);
        }
        self.clock = Some(clock);
        self.raise_clock_floor();
        result?;
        self.view(&mut Work::new(&work.narrow(&output)?)?)
    }
    pub(crate) fn cancel_run(&mut self, c: &OperationContext) -> Result<View> {
        let result = self.cancel_clock_inner(c);
        self.raise_clock_floor();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    fn cancel_clock_inner(&mut self, c: &OperationContext) -> Result<View> {
        // Safety cancellation has Query + ControlClock authority. It does not
        // require Observe, Plan, unfinished goal time or a current goal sample.
        self.check_clock_control(c, false)?;
        self.clock_review = None;
        let mut work = Work::new(c)?;
        let output = work.take(256 * MIB)?;
        let error_view = work.take(CLOCK_VIEW_BYTES)?;
        let target = self.inspect_original_clock_target(&work.take(GUARDED_TARGET_BYTES)?)?;
        self.refresh_clock_view(&work.take(CLOCK_VIEW_BYTES)?)?;
        let needs_source = self
            .clock
            .as_ref()
            .and_then(|clock| clock.latest.as_ref())
            .is_some_and(ClockView::cancellation_needs_source);
        let mut source = if needs_source {
            Some(self.config.orders.clock_source(&work.take(4 * MIB)?)?)
        } else {
            None
        };
        self.clock_invalidates_goal = true;
        self.parent_goal = None;
        let clock = self
            .clock
            .as_mut()
            .ok_or_else(|| denied("clock owner absent"))?;
        clock.inspection_required = true;
        let result = clock
            .inner
            .cancel(&target, source.as_mut(), &work.take(CLOCK_ACTION_BYTES)?);
        if let Ok(view) = &result {
            clock.latest = Some(view.clone());
            clock.inspection_required = false;
        } else if let Ok(current) = work.narrow(&error_view) {
            let _ = clock.inspect(&current);
        }
        self.raise_clock_floor();
        result?;
        self.view(&mut Work::new(&work.narrow(&output)?)?)
    }
}
