//! Complete an already observed original inventory stability window even when
//! its native order has retired. Run/1.13 owns bounded time and single-attempt
//! unpause; this parent owns the exact original plan, predecessor custody and
//! fresh joined labor/inventory evidence at every effect boundary.
use super::*;
use dfmcp_adapter::bounded_run::journal::{RunJournal, RunState};
use dfmcp_adapter::bounded_run::private_file::PrivateRunFile;
use dfmcp_adapter::bounded_run::rpc::RunSource;
use dfmcp_adapter::live_production::StepEvidence;
use dfmcp_adapter::semantic_work_orders::ObservedInventoryObjective;
use dfmcp_adapter::work_order_control::CreationState;
use dfmcp_intent::ObligationStatus;
use dfmcp_world::PredicateTruth;
use orders::inventory_clock::{Clock, ClockTarget, ClockView, RunRequest};

pub(super) struct ProductionInventoryClock {
    inner: Clock<PrivateRunFile>,
    target: Option<ClockTarget>,
    latest: Option<ClockView>,
    inspection_required: bool,
}
impl ProductionInventoryClock {
    pub(super) fn new(journal: RunJournal<PrivateRunFile>) -> Self {
        Self {
            inner: Clock::new(journal),
            target: None,
            latest: None,
            inspection_required: false,
        }
    }
    pub(super) fn needs_target(&self) -> bool {
        self.target.is_none()
    }
    pub(super) fn bind_target(&mut self, target: ClockTarget) -> Result<()> {
        if self
            .target
            .as_ref()
            .is_some_and(|retained| retained.digest() != target.digest())
        {
            return Err(denied(
                "inventory clock cannot replace its original selected goal",
            ));
        }
        self.target = Some(target);
        Ok(())
    }
    fn inspect(&mut self, c: &OperationContext, n: &OperationContext) -> Result<ClockView> {
        self.inspection_required = true;
        let view = self.inner.inspect(self.target.as_ref(), c, n)?;
        self.latest = Some(view.clone());
        self.inspection_required = false;
        Ok(view)
    }
    pub(super) fn high_tick(&self) -> u64 {
        self.inner
            .high_tick()
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
                .is_some_and(|tick| {
                    run.entry
                        .plan()
                        .before()
                        .tick()
                        .is_some_and(|before| tick > before)
                })
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
            value["source"] = json!("run/1.13");
            value["target_scope"] = json!("selected_original_inventory_goal");
            value["independent_authority"] = json!(true);
            value["original_deadline_renewed"] = json!(false);
            value["automatic_dispatch"] = json!(false);
            value["inspection_required"] = json!(self.inspection_required);
            if self.inspection_required {
                value["pending"] = json!(true);
                value["unresolved"] = json!(true);
                value["may_be_running"] = json!(true);
                value["needs_reconciliation"] = json!(true);
            }
            value
        })
    }
}

const VIEW_BYTES: u64 = 8 * MIB;
const ACTION_BYTES: u64 = 96 * MIB;
const TARGET_BYTES: u64 = 1600 * MIB;
// Custody, two current labor reads and the selected inventory sample are all
// prepaid before the durable native marker. These are work bounds, not buffers.
const GUARDED_SAMPLE_BYTES: u64 = 5120 * MIB;

impl Runtime {
    pub(crate) fn inventory_clock_control_enabled(&self) -> bool {
        self.opening_inventory_clock
            && self.mode == Mode::Control
            && self.config.orders.inventory_clock_enabled()
            && self
                .inventory_clock
                .as_ref()
                .is_some_and(|clock| clock.inner.is_control())
    }
    pub(super) fn inventory_clock_json(&self) -> Value {
        self.inventory_clock.as_ref().map_or(Value::Null, |clock| {
            let mut value = clock.to_json();
            if value.is_object() {
                value["review_available"] = json!(
                    !clock.inspection_required
                        && self.inventory_clock_review.is_some_and(|seal| {
                            clock
                                .latest
                                .as_ref()
                                .and_then(ClockView::active)
                                .is_some_and(|run| {
                                    run.review_seal == seal
                                        && run.entry.state() == RunState::Prepared
                                        && run.entry.plan().before().tick()
                                            == Some(self.anchor.tick.get())
                                })
                        })
                );
            }
            value
        })
    }
    pub(super) fn bind_created_inventory_clock_target(&mut self, settled: bool) -> Result<()> {
        if !settled
            || self
                .inventory_clock
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
        if Some(result.review().step_id()) != self.selected_order_step()
            || ObservedInventoryObjective::from_selected_plan(original, result.review().step_id())
                .is_none()
        {
            return Ok(());
        }
        let target = ClockTarget::for_selected_original(
            original,
            self.seal
                .ok_or_else(|| invalid("original archive absent"))?,
            self.order_journal,
            result,
        )?;
        self.inventory_clock
            .as_mut()
            .ok_or_else(|| invalid("inventory clock owner absent"))?
            .bind_target(target)
    }
    pub(super) fn restore_inventory_clock_target(&mut self, work: &mut Work) -> Result<()> {
        if self
            .inventory_clock
            .as_ref()
            .is_none_or(|clock| clock.target.is_some())
        {
            return Ok(());
        }
        let Some(original) = &self.original else {
            return Ok(());
        };
        if !self.selected_order_step().is_some_and(|step| {
            ObservedInventoryObjective::from_selected_plan(original, step).is_some()
        }) {
            return Ok(());
        }
        let ready = self
            .frontier
            .as_mut()
            .ok_or_else(|| denied("original frontier absent"))?
            .inspect(&work.take(VIEW_BYTES)?)?;
        if !ready.all_applied() || ready.needs_reconciliation() {
            return Ok(());
        }
        self.bind_created_inventory_clock_target(true)?;
        if self
            .inventory_clock
            .as_ref()
            .is_some_and(|clock| clock.target.is_none())
        {
            let target = self.inspect_original_goal_clock_target(
                &work.take(TARGET_BYTES)?,
                ClockTarget::for_selected_original,
            )?;
            self.inventory_clock
                .as_mut()
                .ok_or_else(|| denied("inventory clock owner absent"))?
                .bind_target(target)?;
        }
        Ok(())
    }
    pub(super) fn refresh_inventory_clock_view(&mut self, c: &OperationContext) -> Result<()> {
        if self.inventory_clock.is_none() {
            return Ok(());
        }
        let native = self.config.orders.inventory_clock_context(c, false)?;
        let result = self
            .inventory_clock
            .as_mut()
            .ok_or_else(|| denied("inventory clock owner absent"))?
            .inspect(c, &native);
        self.raise_clock_floor();
        result.map(|_| ())
    }
    pub(super) fn reconcile_inventory_clock(&mut self, work: &mut Work) -> Result<()> {
        if self.inventory_clock.is_none() {
            return Ok(());
        }
        if self.portfolio.is_some()
            && self
                .inventory_clock
                .as_ref()
                .is_some_and(|clock| !clock.has_runs())
        {
            self.refresh_inventory_clock_view(&work.take(VIEW_BYTES)?)?;
            if self
                .inventory_clock
                .as_ref()
                .is_some_and(|clock| !clock.has_runs())
            {
                return Ok(());
            }
        }
        self.restore_inventory_clock_target(work)?;
        self.refresh_inventory_clock_view(&work.take(VIEW_BYTES)?)?;
        if !self
            .inventory_clock
            .as_ref()
            .and_then(|clock| clock.latest.as_ref())
            .is_some_and(ClockView::needs_reconciliation)
        {
            return Ok(());
        }
        let source_context = self
            .config
            .orders
            .inventory_clock_context(&work.take(4 * MIB)?, false)?;
        let current = work.take(ACTION_BYTES)?;
        let native = self
            .config
            .orders
            .inventory_clock_context(&current, false)?;
        let error_view = work.take(VIEW_BYTES)?;
        let mut source = self.config.orders.inventory_clock_source(&source_context)?;
        let clock = self
            .inventory_clock
            .as_mut()
            .ok_or_else(|| denied("inventory clock owner absent"))?;
        let target = clock
            .target
            .as_ref()
            .ok_or_else(|| denied("original inventory clock target absent"))?;
        clock.inspection_required = true;
        let result = clock
            .inner
            .reconcile(target, &mut source, &work.narrow(&current)?, &native);
        if let Ok(view) = &result {
            clock.latest = Some(view.clone());
            clock.inspection_required = false;
        } else if let Ok(current) = work.narrow(&error_view)
            && let Ok(native) = self.config.orders.inventory_clock_context(&current, false)
        {
            let _ = clock.inspect(&current, &native);
        }
        self.raise_clock_floor();
        result.map(|_| ())
    }
    fn check_inventory_clock_control(&self, c: &OperationContext, planning: bool) -> Result<()> {
        self.check(c, planning)?;
        if !self.inventory_clock_control_enabled() {
            return Err(denied(
                "this combined owner has no current independent inventory clock authority",
            ));
        }
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
    }
    fn inventory_clock_goal_ready(&self) -> Result<()> {
        if self.clock.as_ref().is_some_and(ProductionClock::pending) {
            return Err(denied(
                "settle or cancel the retained order clock before an inventory run",
            ));
        }
        let monitor = self.goal_monitor.as_ref().ok_or_else(|| denied("observe original inventory stability with fortress.wait before requesting inventory_run"))?;
        let sample = monitor
            .latest()
            .filter(|sample| {
                sample.anchor.tick == self.anchor.tick
                    && (self.portfolio.is_some() || sample.anchor.cursor == self.anchor.cursor)
                    && sample.current_goal == PredicateTruth::True
                    && sample.failure_predicate != Some(PredicateTruth::True)
            })
            .ok_or_else(|| {
                denied("inventory clock requires a fresh qualified True inventory sample")
            })?;
        if self.parent_goal.as_ref().is_none_or(|parent| {
            parent.anchor != self.anchor
                || parent.selected_anchor != sample.anchor
                || parent.current_truth != PredicateTruth::True
        }) || self.clock_withholds_goal()
        {
            return Err(denied(
                "inventory clock requires fresh True original labor and inventory evidence",
            ));
        }
        if !matches!(
            monitor.historical_obligation_status(),
            Some(ObligationStatus::Pending | ObligationStatus::Active { .. })
        ) {
            return Err(denied(
                "inventory clock requires unfinished original stability, without a new deadline",
            ));
        }
        Ok(())
    }
    fn sample_inventory_clock_target(
        &mut self,
        c: &OperationContext,
        refresh: bool,
    ) -> Result<ClockTarget> {
        // Initialization/history replay has its own bounded Wait operation.
        // Neither historical receipts nor restored goal history start a run.
        if self
            .goal_monitor
            .as_ref()
            .and_then(|monitor| monitor.latest())
            .is_none()
        {
            return Err(denied(
                "observe original inventory stability with fortress.wait before requesting inventory_run",
            ));
        }
        if refresh {
            self.wait_with_goal(c)?;
        }
        self.inventory_clock_goal_ready()?;
        let clock = self
            .inventory_clock
            .as_ref()
            .ok_or_else(|| denied("inventory clock owner absent"))?;
        if clock.may_be_running() {
            return Err(denied(
                "reconcile or cancel the retained inventory clock before another run",
            ));
        }
        clock
            .target
            .clone()
            .ok_or_else(|| denied("inventory clock requires exact original Created custody"))
    }
    fn verify_inventory_clock_edge(
        &mut self,
        target: &ClockTarget,
        c: &OperationContext,
    ) -> Result<()> {
        self.config.current()?;
        if !self.opening_inventory_clock || !self.config.orders.inventory_clock_enabled() {
            return Err(denied("independent inventory clock authority was revoked"));
        }
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        let mut work = Work::new(c)?;
        self.verify_other_portfolio_clocks(&mut work)?;
        let current = self.inspect_original_goal_clock_target(
            &work.take(TARGET_BYTES)?,
            ClockTarget::for_selected_original,
        )?;
        if current.digest() != target.digest() {
            return Err(denied(
                "original inventory clock custody changed before its native effect",
            ));
        }
        self.wait_with_goal(&work.rest(MAX_BYTES)?)?;
        self.inventory_clock_goal_ready()?;
        if self.anchor.tick != c.anchor.tick {
            return Err(denied(
                "inventory clock's fresh goal evidence changed tick before unpause",
            ));
        }
        self.config.current()?;
        work.current()?;
        Ok(())
    }
    pub(crate) fn plan_inventory_run(
        &mut self,
        request: RunRequest,
        c: &OperationContext,
    ) -> Result<View> {
        self.run_inventory_clock(
            c,
            |clock, target, source, canonical, native, guard| {
                clock.prepare_guarded(target, request.inner(), source, canonical, native, guard)
            },
            false,
        )
    }
    pub(crate) fn commit_inventory_run(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
    ) -> Result<View> {
        if !confirmed || self.inventory_clock_review != Some(seal) {
            self.inventory_clock_review = None;
            return Err(denied(
                "prepare and confirm this owner's exact original inventory clock review",
            ));
        }
        self.inventory_clock_review = None;
        self.run_inventory_clock(
            c,
            |clock, target, source, canonical, native, guard| {
                clock.commit_guarded(target, seal, true, source, canonical, native, guard)
            },
            true,
        )
    }
    fn run_inventory_clock(
        &mut self,
        c: &OperationContext,
        action: impl FnOnce(
            &mut Clock<PrivateRunFile>,
            &ClockTarget,
            &mut orders::native::PinnedInventoryClock,
            &OperationContext,
            &OperationContext,
            &mut dyn FnMut(&OperationContext) -> Result<()>,
        ) -> Result<ClockView>,
        advancing: bool,
    ) -> Result<View> {
        let result = self.run_inventory_clock_inner(c, action, advancing);
        self.raise_clock_floor();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    fn run_inventory_clock_inner(
        &mut self,
        c: &OperationContext,
        action: impl FnOnce(
            &mut Clock<PrivateRunFile>,
            &ClockTarget,
            &mut orders::native::PinnedInventoryClock,
            &OperationContext,
            &OperationContext,
            &mut dyn FnMut(&OperationContext) -> Result<()>,
        ) -> Result<ClockView>,
        advancing: bool,
    ) -> Result<View> {
        self.check_inventory_clock_control(c, true)?;
        self.clear_portfolio_cohort();
        let mut work = Work::new(c)?;
        self.refresh_inventory_clock_view(&work.take(VIEW_BYTES)?)?;
        if self.clock.is_some() {
            self.refresh_clock_view(&work.take(VIEW_BYTES)?)?;
        }
        // A new native effect always requalifies the complete parent after its
        // durable marker. Do that once, using the prior explicit Wait sample as
        // the required matching source tick. Re-reviewing an existing Prepared
        // run has no native effect/guard, so refresh its evidence here instead.
        let reviewing = !advancing
            && self
                .inventory_clock
                .as_ref()
                .and_then(|clock| clock.latest.as_ref())
                .and_then(ClockView::active)
                .is_some_and(|run| run.entry.state() == RunState::Prepared);
        let output = work.take(256 * MIB)?;
        let source_context = work.take(4 * MIB)?;
        let mut guard_context = if reviewing {
            None
        } else {
            Some(work.take(GUARDED_SAMPLE_BYTES)?)
        };
        let run_context = work.take(ACTION_BYTES)?;
        let error_view = work.take(VIEW_BYTES)?;
        if self.portfolio.is_some()
            && self
                .inventory_clock
                .as_ref()
                .is_some_and(|clock| clock.target.is_none())
        {
            self.restore_inventory_clock_target(&mut work)?;
        }
        let target = self.sample_inventory_clock_target(&work.rest(MAX_BYTES)?, reviewing)?;
        work.context.anchor = self.anchor;
        let source_context = self
            .config
            .orders
            .inventory_clock_context(&work.narrow(&source_context)?, true)?;
        let mut source = self.config.orders.inventory_clock_source(&source_context)?;
        let canonical = work.narrow(&run_context)?;
        let native = self
            .config
            .orders
            .inventory_clock_context(&canonical, true)?;
        let mut clock = self
            .inventory_clock
            .take()
            .ok_or_else(|| denied("inventory clock owner absent"))?;
        clock.inspection_required = true;
        if advancing {
            self.withdraw_portfolio_current();
            self.inventory_clock_invalidates_goal = true;
            self.parent_goal = None;
        }
        let result = action(
            &mut clock.inner,
            &target,
            &mut source,
            &canonical,
            &native,
            &mut |edge| {
                let mut guarded = guard_context
                    .take()
                    .ok_or_else(|| denied("original inventory clock guard cannot repeat"))?;
                guarded.anchor = edge.anchor;
                guarded.budget.max_wall_millis = guarded
                    .budget
                    .max_wall_millis
                    .min(edge.budget.max_wall_millis);
                self.verify_inventory_clock_edge(&target, &guarded)
            },
        );
        // The guard observes before unpause. It cannot publish a current proof
        // after an attempted advancing effect, including a lost native reply.
        if advancing {
            self.withdraw_portfolio_current();
            self.inventory_clock_invalidates_goal = true;
            self.parent_goal = None;
            if let Some(basis) = &mut self.basis {
                basis.choices = None;
            }
        }
        if let Ok(view) = &result {
            clock.latest = Some(view.clone());
            clock.inspection_required = false;
            if !advancing {
                self.inventory_clock_review = view
                    .active()
                    .filter(|run| run.entry.state() == RunState::Prepared)
                    .map(|run| run.review_seal);
            }
        } else if let Ok(current) = work.narrow(&error_view)
            && let Ok(native) = self.config.orders.inventory_clock_context(&current, false)
        {
            let _ = clock.inspect(&current, &native);
        }
        self.inventory_clock = Some(clock);
        self.raise_clock_floor();
        result?;
        work.context.anchor = self.anchor;
        self.view(&mut Work::new(&work.narrow(&output)?)?)
    }
    pub(crate) fn cancel_inventory_run(&mut self, c: &OperationContext) -> Result<View> {
        let result = self.cancel_inventory_clock_inner(c);
        self.raise_clock_floor();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    fn cancel_inventory_clock_inner(&mut self, c: &OperationContext) -> Result<View> {
        // Safety cancellation has Query + independent ControlClock authority;
        // it never requires Plan, Observe, current truth or remaining goal time.
        self.check_inventory_clock_control(c, false)?;
        self.inventory_clock_review = None;
        let mut work = Work::new(c)?;
        let output = work.take(256 * MIB)?;
        let error_view = work.take(VIEW_BYTES)?;
        let target = self.inspect_original_goal_clock_target(
            &work.take(TARGET_BYTES)?,
            ClockTarget::for_selected_original,
        )?;
        self.refresh_inventory_clock_view(&work.take(VIEW_BYTES)?)?;
        let needs_source = self
            .inventory_clock
            .as_ref()
            .and_then(|clock| clock.latest.as_ref())
            .is_some_and(ClockView::cancellation_needs_source);
        let mut source = if needs_source {
            let native = self
                .config
                .orders
                .inventory_clock_context(&work.take(4 * MIB)?, true)?;
            Some(self.config.orders.inventory_clock_source(&native)?)
        } else {
            None
        };
        let canonical = work.take(ACTION_BYTES)?;
        let native = self
            .config
            .orders
            .inventory_clock_context(&canonical, true)?;
        self.inventory_clock_invalidates_goal = true;
        self.withdraw_portfolio_current();
        self.parent_goal = None;
        let clock = self
            .inventory_clock
            .as_mut()
            .ok_or_else(|| denied("inventory clock owner absent"))?;
        clock.inspection_required = true;
        let result = clock.inner.cancel(
            &target,
            source.as_mut().map(|source| source as &mut dyn RunSource),
            &canonical,
            &native,
        );
        if let Ok(view) = &result {
            clock.latest = Some(view.clone());
            clock.inspection_required = false;
        } else if let Ok(current) = work.narrow(&error_view)
            && let Ok(native) = self.config.orders.inventory_clock_context(&current, false)
        {
            let _ = clock.inspect(&current, &native);
        }
        self.raise_clock_floor();
        result?;
        self.view(&mut Work::new(&work.narrow(&output)?)?)
    }
}
