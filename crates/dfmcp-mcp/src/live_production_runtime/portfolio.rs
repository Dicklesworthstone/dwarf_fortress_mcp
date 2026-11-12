//! Bounded foreground portfolio owner. One Wait samples one original leaf; a
//! complete progress bracket spans the cohort and is never restored by replay.
use super::*;
use dfmcp_adapter::live_production::{ProductionPortfolioGoalProgress, SelectedParentGoalResult};
use dfmcp_adapter::work_order_progress::{ProgressObservation, ProgressSource};
use dfmcp_core::{GameTick, StateAnchor, StepId};
use dfmcp_world::PredicateTruth;
use std::collections::BTreeMap;

// Both existing clock journals are capped at 2 MiB. The order opener pays
// <=3 MiB bootstrap +3 MiB final view +2 MiB file read. The inventory opener
// pays 8 MiB final records view plus <=2 MiB read +2 MiB replay verification.
// Six additional fixed slots therefore fit without reducing any per-target
// custody check or borrowing the separately reserved response allowance.
const CLOCK_OPEN_BYTES: u64 = 16 * MIB;

/// Current whole-target proof has its own bounded sampling window. A target
/// with no action cannot borrow another leaf's historical stability. This
/// process-local state is never reconstructed from separate leaf histories.
#[derive(Clone, Debug)]
struct ParentStability {
    original_anchor: StateAnchor,
    deadline_tick: GameTick,
    poll_interval_ticks: u64,
    required_samples: u32,
    accepted_samples: u32,
    first_candidate_tick: Option<GameTick>,
    fulfilled_at_tick: Option<GameTick>,
    last_sample_tick: Option<GameTick>,
    last_evaluated_tick: Option<GameTick>,
    last_anchor: Option<StateAnchor>,
    expired: bool,
}

impl ParentStability {
    fn new(
        original_anchor: StateAnchor,
        deadline_tick: GameTick,
        poll_interval_ticks: u64,
        required_samples: u32,
    ) -> Result<Self> {
        if deadline_tick <= original_anchor.tick
            || poll_interval_ticks == 0
            || required_samples == 0
        {
            return Err(denied("whole-target stability requires the original finite timing bounds"));
        }
        Ok(Self {
            original_anchor,
            deadline_tick,
            poll_interval_ticks,
            required_samples,
            accepted_samples: 0,
            first_candidate_tick: None,
            fulfilled_at_tick: None,
            last_sample_tick: None,
            last_evaluated_tick: None,
            last_anchor: None,
            expired: false,
        })
    }

    fn for_plan(original: &PreparedPlan) -> Result<Self> {
        let spec = original.steps.first().and_then(|step| step.obligation.as_ref())
            .ok_or_else(|| denied("whole-target stability lacks its original obligation"))?;
        if original.steps.iter().any(|step| step.obligation.as_ref().is_none_or(|other| {
            other.deadline_tick != spec.deadline_tick
                || other.poll_interval_ticks != spec.poll_interval_ticks
                || other.stable_for_observations != spec.stable_for_observations
                || other.failure.is_some()
        })) {
            return Err(denied("whole-target stability must retain one original timing contract"));
        }
        Self::new(
            original.anchor, spec.deadline_tick, spec.poll_interval_ticks,
            spec.stable_for_observations,
        )
    }

    fn fulfilled(&self) -> bool {
        self.accepted_samples >= self.required_samples
    }

    fn interrupt(&mut self) {
        if !self.fulfilled() {
            self.accepted_samples = 0;
            self.first_candidate_tick = None;
            self.last_sample_tick = None;
        }
    }

    fn needs_time(&self, anchor: StateAnchor) -> bool {
        !self.expired && !self.fulfilled()
            && anchor.fortress_id == self.original_anchor.fortress_id
            && anchor.cursor.epoch == self.original_anchor.cursor.epoch
            && anchor.tick >= self.original_anchor.tick
            && anchor.tick < self.deadline_tick
    }

    fn observe(&mut self, anchor: StateAnchor, truth: PredicateTruth) -> Result<bool> {
        if anchor.fortress_id != self.original_anchor.fortress_id
            || anchor.cursor.epoch != self.original_anchor.cursor.epoch
            || anchor.tick < self.original_anchor.tick
            || anchor.cursor.sequence < self.original_anchor.cursor.sequence
            || self.last_anchor.is_some_and(|last| {
                anchor.tick < last.tick || anchor.cursor.sequence < last.cursor.sequence
                    || (anchor.cursor == last.cursor && anchor != last)
            })
        {
            self.interrupt();
            return Err(denied("whole-target stability source regressed or changed lineage"));
        }
        self.last_anchor = Some(anchor);
        // Like the original leaf histories, an achieved temporal requirement
        // remains historical. Fresh current truth is still mandatory.
        if self.fulfilled() {
            self.last_sample_tick = Some(anchor.tick);
            return Ok(truth == PredicateTruth::True);
        }
        if self.expired || anchor.tick > self.deadline_tick {
            self.interrupt();
            self.expired = true;
            return Ok(false);
        }
        if truth != PredicateTruth::True {
            self.interrupt();
        }
        let basis = self.last_evaluated_tick.unwrap_or(self.original_anchor.tick);
        let due = self.last_evaluated_tick != Some(anchor.tick)
            && anchor.tick.get().saturating_sub(basis.get()) >= self.poll_interval_ticks;
        if due {
            self.last_evaluated_tick = Some(anchor.tick);
            if truth == PredicateTruth::True {
                self.accepted_samples = self.accepted_samples.saturating_add(1);
                if self.first_candidate_tick.is_none() {
                    self.first_candidate_tick = Some(anchor.tick);
                }
            }
        }
        if self.accepted_samples > 0 {
            self.last_sample_tick = Some(anchor.tick);
        }
        if self.fulfilled() {
            self.fulfilled_at_tick = Some(anchor.tick);
        }
        if !self.fulfilled() && anchor.tick >= self.deadline_tick {
            self.interrupt();
            self.expired = true;
        }
        Ok(self.fulfilled() && truth == PredicateTruth::True)
    }

    fn to_json(&self, proven: bool) -> Value {
        json!({
            "required_samples":self.required_samples,
            "accepted_samples":self.accepted_samples,
            "poll_interval_ticks":self.poll_interval_ticks,
            "deadline_tick":self.deadline_tick.get(),
            "first_candidate_tick":self.first_candidate_tick.map(GameTick::get),
            "fulfilled_at_tick":self.fulfilled_at_tick.map(GameTick::get),
            "last_sample_tick":self.last_sample_tick.map(GameTick::get),
            "status":if self.fulfilled() { "fulfilled" } else if self.expired { "expired" } else { "pending" },
            "proven_at_anchor":proven && self.fulfilled(),
            "restored_from_history":false
        })
    }
}

#[derive(Default)]
struct Leaf {
    monitor: Option<SelectedWorkOrderGoalMonitor<File>>,
    history_present: bool,
    current_withdrawn: bool,
    parent: Option<SelectedParentGoalProgress>,
    inspection: Option<InspectionSource>,
    clock: Option<ProductionClock>,
    clock_invalidates: bool,
    clock_review: Option<Digest32>,
    inventory_clock: Option<ProductionInventoryClock>,
    inventory_invalidates: bool,
    inventory_review: Option<Digest32>,
    goal_json: Value,
}
pub(super) struct PortfolioState {
    active: StepId,
    leaves: BTreeMap<StepId, Leaf>,
    cohort: Option<(ProgressObservation, Vec<SelectedParentGoalResult>)>,
    pub(super) current: Option<ProductionPortfolioGoalProgress>,
    parent_stability: Option<ParentStability>,
}
fn cohort_covers_selected(
    parent: &ProductionPortfolioGoalProgress,
    current_anchor: StateAnchor,
    step: StepId,
    selected_anchor: StateAnchor,
) -> bool {
    parent.anchor == current_anchor && parent.current_truth == PredicateTruth::True
        && parent.selected_anchors.contains(&(step, selected_anchor))
}

impl Runtime {
    pub(super) fn selected_order_step(&self) -> Option<StepId> {
        self.portfolio.as_ref().map(|p| p.active).or_else(||
            self.layout.as_ref().and_then(|layout| layout.order_steps().first().copied()))
    }
    fn portfolio_ids(&self) -> Vec<StepId> {
        self.layout.as_ref().map_or_else(Vec::new, |layout| layout.order_steps().to_vec())
    }
    /// Private filenames use fixed one-based slots, not original StepIds.
    /// Both the old labor predecessor layout and the dependency-free layout
    /// retain their actual original StepIds everywhere else.
    pub(super) fn order_slot(&self, step: StepId) -> Result<u32> {
        self.layout.as_ref().and_then(|layout| layout.order_ordinal(step))
            .and_then(|ordinal| u32::try_from(ordinal + 1).ok())
            .ok_or_else(|| denied("selected step is not in the original production layout"))
    }
    pub(super) fn initialize_portfolio(
        &mut self,
        recovery: Option<[Option<config::PortfolioRecovery>; 4]>,
        work: &mut Work,
    ) -> Result<()> {
        let ids = self.portfolio_ids();
        if ids.is_empty()
            || ids.len() > 4
            || self.portfolio.is_some()
            || recovery.is_some_and(|r| r.iter().filter(|r| r.is_some()).count() != ids.len())
            || recovery.is_some_and(|r| ids.iter().enumerate().any(|(index, step)|
                r[index].is_none_or(|entry| entry.step_id != step.get())))
        {
            return Err(denied(
                "portfolio custody does not match every original order leaf",
            ));
        }
        self.config
            .orders
            .require_absent_portfolio_clocks(ids.len() as u32 + 1)?;
        let parent_stability = if self.layout.as_ref()
            .and_then(|layout| layout.existing_labor())
            .is_some_and(|requirement| !requirement.inventory_minima().is_empty())
        {
            Some(ParentStability::for_plan(self.original.as_ref()
                .ok_or_else(|| denied("original stock-target portfolio absent"))?)?)
        } else {
            None
        };
        self.portfolio = Some(PortfolioState {
            active: ids[0],
            leaves: BTreeMap::new(),
            cohort: None,
            current: None,
            parent_stability,
        });
        if let Some(first) = recovery.and_then(|r| r[0]) {
            self.goal_history_present = first.goal_history_present;
        }
        for (ordinal, step) in ids.iter().copied().enumerate().skip(1) {
            let retained = recovery.and_then(|r| r[ordinal]);
            let slot = self.order_slot(step)?;
            let clock = self
                .config
                .orders
                .open_portfolio_clock(
                    slot,
                    order_mode(self.mode),
                    recovery.is_none(),
                    retained.and_then(|r| r.clock_journal_id),
                    &work.take(CLOCK_OPEN_BYTES)?,
                )?
                .map(ProductionClock::new);
            let inventory_clock = self
                .config
                .orders
                .open_portfolio_inventory_clock(
                    slot,
                    order_mode(self.mode),
                    recovery.is_none(),
                    retained.and_then(|r| r.inventory_clock_journal_id),
                    &work.take(CLOCK_OPEN_BYTES)?,
                )?
                .map(ProductionInventoryClock::new);
            self.portfolio
                .as_mut()
                .ok_or_else(|| denied("portfolio absent"))?
                .leaves
                .insert(
                    step,
                    Leaf {
                        clock,
                        inventory_clock,
                        history_present: retained.is_some_and(|r| r.goal_history_present),
                        ..Leaf::default()
                    },
                );
            self.activate_leaf(step)?;
            if recovery.is_none() {
                self.refresh_clock_view(&work.take(8 * MIB)?)?;
                self.refresh_inventory_clock_view(&work.take(8 * MIB)?)?;
            }
        }
        self.activate_leaf(ids[0])?;
        if recovery.is_some() {
            self.restore_portfolio_targets(work)?;
        }
        Ok(())
    }
    /// Replay every fixed journal in one operation. Parent archive/identity
    /// reads share one armed bounded allowance; each selected insertion still
    /// receives its own complete executor/dependency verification. The two
    /// independently authorized clock families may reuse only that immutable
    /// selected identity, never a preparation or effect permit.
    fn restore_portfolio_targets(&mut self, work: &mut Work) -> Result<()> {
        let active = self
            .selected_order_step()
            .ok_or_else(|| denied("portfolio selection absent"))?;
        let ready = self
            .frontier
            .as_mut()
            .ok_or_else(|| denied("portfolio frontier absent"))?
            .inspect(&work.take(8 * MIB)?)?;
        self.arm(work)?;
        let custody = self
            .custody
            .as_ref()
            .ok_or_else(|| denied("portfolio custody absent"))?
            .clone();
        let result = (|| {
            custody.verify(&work.current()?)?;
            for step in self.portfolio_ids() {
                self.activate_leaf(step)?;
                let order = self
                    .clock
                    .as_ref()
                    .is_some_and(ProductionClock::needs_target);
                let inventory = self.inventory_clock.as_ref().is_some_and(ProductionInventoryClock::needs_target)
                    && self.original.as_ref().is_some_and(|p|
                        dfmcp_adapter::semantic_work_orders::ObservedInventoryGoal::from_selected_plan(p, step).is_some());
                if ready.all_applied() && !ready.needs_reconciliation() && (order || inventory) {
                    // Existing single-target custody admission is 1024 MiB.
                    // Four exact leaves cost 4*1024 MiB, not one replenished
                    // allowance per family; parent custody is paid once above.
                    let inspected = self
                        .execution
                        .as_mut()
                        .ok_or_else(|| denied("portfolio executor absent"))?
                        .inspect_selected_order_custody(step, &work.take(1024 * MIB)?)?;
                    let original = self
                        .original
                        .as_ref()
                        .ok_or_else(|| denied("original portfolio absent"))?;
                    let seal = self.seal.ok_or_else(|| denied("original archive absent"))?;
                    if order {
                        let target = orders::clock::ClockTarget::for_selected_original(
                            original,
                            seal,
                            self.order_journal,
                            &inspected,
                        )?;
                        self.clock
                            .as_mut()
                            .ok_or_else(|| denied("order clock absent"))?
                            .bind_target(target)?;
                    }
                    if inventory {
                        let target = orders::inventory_clock::ClockTarget::for_selected_original(
                            original,
                            seal,
                            self.order_journal,
                            &inspected,
                        )?;
                        self.inventory_clock
                            .as_mut()
                            .ok_or_else(|| denied("inventory clock absent"))?
                            .bind_target(target)?;
                    }
                }
                self.refresh_clock_view(&work.take(8 * MIB)?)?;
                self.refresh_inventory_clock_view(&work.take(8 * MIB)?)?;
                self.clock_invalidates_goal =
                    self.clock.as_ref().is_some_and(ProductionClock::has_runs);
                self.inventory_clock_invalidates_goal = self
                    .inventory_clock
                    .as_ref()
                    .is_some_and(ProductionInventoryClock::has_runs);
            }
            custody.verify(&work.current()?)?;
            self.config.current()?;
            Ok(())
        })();
        let disarmed = custody.disarm();
        let restored = self.activate_leaf(active);
        result?;
        disarmed?;
        restored
    }
    fn activate_leaf(&mut self, step: StepId) -> Result<()> {
        let Some(portfolio) = self.portfolio.as_ref() else {
            return if self.selected_order_step() == Some(step) {
                Ok(())
            } else {
                Err(denied("original order selector differs"))
            };
        };
        if portfolio.active == step {
            return Ok(());
        }
        if !portfolio.leaves.contains_key(&step) {
            return Err(invalid("order_step_id is not an original portfolio order"));
        }
        let goal_json = self.selected_goal_json();
        let portfolio = self
            .portfolio
            .as_mut()
            .ok_or_else(|| denied("portfolio absent"))?;
        let next = portfolio
            .leaves
            .remove(&step)
            .ok_or_else(|| denied("portfolio slot absent"))?;
        let old = Leaf {
            monitor: self.goal_monitor.take(),
            history_present: self.goal_history_present,
            current_withdrawn: self.goal_current_withdrawn,
            parent: self.parent_goal.take(),
            inspection: self.inspection_source.take(),
            clock: self.clock.take(),
            clock_invalidates: self.clock_invalidates_goal,
            clock_review: self.clock_review.take(),
            inventory_clock: self.inventory_clock.take(),
            inventory_invalidates: self.inventory_clock_invalidates_goal,
            inventory_review: self.inventory_clock_review.take(),
            goal_json,
        };
        portfolio.leaves.insert(portfolio.active, old);
        portfolio.active = step;
        self.goal_monitor = next.monitor;
        self.goal_history_present = next.history_present;
        self.goal_current_withdrawn = next.current_withdrawn;
        self.parent_goal = next.parent;
        self.inspection_source = next.inspection;
        self.clock = next.clock;
        self.clock_invalidates_goal = next.clock_invalidates;
        self.clock_review = next.clock_review;
        self.inventory_clock = next.inventory_clock;
        self.inventory_clock_invalidates_goal = next.inventory_invalidates;
        self.inventory_clock_review = next.inventory_review;
        self.portfolio_sample = None;
        Ok(())
    }
    pub(crate) fn select_order(&mut self, step: Option<u32>, c: &OperationContext) -> Result<()> {
        self.check(c, false)?;
        if self.portfolio.is_none() {
            return if step.is_none() {
                Ok(())
            } else {
                Err(invalid(
                    "order_step_id is only valid for an original portfolio",
                ))
            };
        }
        let step = StepId::new(step.ok_or_else(|| {
            invalid("portfolio clock calls require the exact original order_step_id")
        })?);
        let pending = self.pending_portfolio_steps();
        if self.selected_order_step() != Some(step) && !pending.is_empty() && pending != [step] {
            return Err(denied(
                "settle the retained original portfolio clock before selecting another order",
            ));
        }
        self.activate_leaf(step)
    }
    pub(super) fn any_portfolio_clock_pending(&self) -> bool {
        self.clock.as_ref().is_some_and(ProductionClock::pending)
            || self
                .inventory_clock
                .as_ref()
                .is_some_and(ProductionInventoryClock::pending)
            || self.portfolio.as_ref().is_some_and(|p| {
                p.leaves.values().any(|leaf| {
                    leaf.clock.as_ref().is_some_and(ProductionClock::pending)
                        || leaf
                            .inventory_clock
                            .as_ref()
                            .is_some_and(ProductionInventoryClock::pending)
                })
            })
    }
    fn pending_portfolio_steps(&self) -> Vec<StepId> {
        let mut ids = Vec::new();
        if let Some(p) = &self.portfolio {
            if self.clock.as_ref().is_some_and(ProductionClock::pending)
                || self
                    .inventory_clock
                    .as_ref()
                    .is_some_and(ProductionInventoryClock::pending)
            {
                ids.push(p.active);
            }
            for (step, leaf) in &p.leaves {
                if leaf.clock.as_ref().is_some_and(ProductionClock::pending)
                    || leaf
                        .inventory_clock
                        .as_ref()
                        .is_some_and(ProductionInventoryClock::pending)
                {
                    ids.push(*step);
                }
            }
            ids.sort_unstable();
        }
        ids
    }
    pub(super) fn verify_other_portfolio_clocks(&mut self, work: &mut Work) -> Result<()> {
        if self.portfolio.is_none() {
            return Ok(());
        }
        let active = self
            .selected_order_step()
            .ok_or_else(|| denied("portfolio selection absent"))?;
        let result = (|| {
            for step in self.portfolio_ids().into_iter().filter(|s| *s != active) {
                self.activate_leaf(step)?;
                self.refresh_clock_view(&work.take(8 * MIB)?)?;
                self.refresh_inventory_clock_view(&work.take(8 * MIB)?)?;
                if self.clock.as_ref().is_some_and(ProductionClock::pending)
                    || self
                        .inventory_clock
                        .as_ref()
                        .is_some_and(ProductionInventoryClock::pending)
                {
                    return Err(denied(
                        "another original portfolio clock is pending or uncertain",
                    ));
                }
            }
            Ok(())
        })();
        let restored = self.activate_leaf(active);
        result?;
        restored
    }
    pub(crate) fn inspect_order(
        &mut self,
        step: Option<u32>,
        c: &OperationContext,
    ) -> Result<View> {
        if step.is_none() {
            return self.inspect(c);
        }
        self.select_order(step, c)?;
        let mut work = Work::new(c)?;
        let output = work.take(256 * MIB)?;
        self.restore_clock_target(&mut work)?;
        self.restore_inventory_clock_target(&mut work)?;
        self.view(&mut Work::new(&work.narrow(&output)?)?)
    }
    pub(super) fn inactive_clock_may_be_running(&self) -> bool {
        self.portfolio.as_ref().is_some_and(|p| {
            p.leaves.values().any(|leaf| {
                leaf.clock
                    .as_ref()
                    .is_some_and(ProductionClock::may_be_running)
                    || leaf
                        .inventory_clock
                        .as_ref()
                        .is_some_and(ProductionInventoryClock::may_be_running)
            })
        })
    }
    /// Time may advance under an original bounded clock without interrupting
    /// the selected histories. Withdraw every current certificate immediately;
    /// the next complete cohort must requalify each leaf at the new tick.
    pub(super) fn withdraw_portfolio_current(&mut self) {
        self.portfolio_sample = None;
        self.inspection_source = None;
        if let Some(p) = &mut self.portfolio {
            self.goal_current_withdrawn = true;
            self.parent_goal = None;
            p.current = None;
            p.cohort = None;
            for leaf in p.leaves.values_mut() {
                leaf.current_withdrawn = true;
                leaf.parent = None;
                leaf.inspection = None;
                leaf.clock_review = None;
                leaf.inventory_review = None;
            }
        }
    }
    /// A failed observation or custody check is an actual history interruption,
    /// unlike a successfully owned clock interval. Preserve the monitor's
    /// existing conservative unfinished-stability reset in this case.
    pub(super) fn invalidate_portfolio_samples(&mut self) {
        self.withdraw_portfolio_current();
        self.interrupt_portfolio_parent_stability();
        if let Some(p) = &mut self.portfolio {
            for leaf in p.leaves.values_mut() {
                if leaf
                    .monitor
                    .as_mut()
                    .is_some_and(|m| m.invalidate_current_sample().is_err())
                {
                    self.fenced = true;
                }
            }
        }
    }
    pub(super) fn interrupt_portfolio_parent_stability(&mut self) {
        if let Some(stability) = self.portfolio.as_mut().and_then(|p| p.parent_stability.as_mut()) {
            stability.interrupt();
        }
    }

    /// A fresh selected sample already contains every original inventory
    /// minimum. A contradiction interrupts the unfinished complete-cohort
    /// streak even during a partial cohort or a guarded clock read.
    pub(super) fn observe_portfolio_parent_truth(&mut self, truth: PredicateTruth) {
        if truth != PredicateTruth::True {
            self.interrupt_portfolio_parent_stability();
        }
    }

    /// A complete cohort can justify a selected clock's initial read boundary
    /// without relabelling its older child anchor as the public composite.
    /// The clock's native effect still acquires a new selected parent sample.
    pub(super) fn portfolio_covers_selected_goal(
        &self, step: StepId, selected_anchor: StateAnchor,
    ) -> bool {
        self.portfolio.as_ref().and_then(|p| p.current.as_ref())
            .is_some_and(|parent| cohort_covers_selected(parent, self.anchor, step, selected_anchor))
    }

    pub(super) fn parent_stability_needs_time(&self) -> bool {
        let pending = self.portfolio.as_ref().and_then(|p| p.parent_stability.as_ref())
            .is_some_and(|stability| stability.needs_time(self.anchor));
        let selected = self.parent_goal.as_ref().is_some_and(|parent| {
            parent.anchor == self.anchor && parent.current_truth == PredicateTruth::True
                && !parent.inventory_requirements.is_empty()
                && parent.inventory_requirements.iter().all(|finding|
                    finding.current_truth == PredicateTruth::True)
        });
        let complete = self.portfolio.as_ref().and_then(|p| p.current.as_ref())
            .is_some_and(|parent| {
                parent.anchor == self.anchor && parent.current_truth == PredicateTruth::True
                    && !parent.inventory_requirements.is_empty()
                    && parent.inventory_requirements.iter().all(|finding|
                        finding.current_truth == PredicateTruth::True)
            });
        pending && (selected || complete)
    }

    pub(super) fn clear_portfolio_cohort(&mut self) {
        self.portfolio_sample = None;
        if let Some(p) = &mut self.portfolio {
            p.current = None;
            p.cohort = None;
        }
    }
    pub(super) fn inactive_portfolio_advancement(&self) -> Vec<Value> {
        self.portfolio.as_ref().map_or_else(Vec::new, |p| {
            p.leaves
                .values()
                .flat_map(|leaf| {
                    [
                        leaf.clock
                            .as_ref()
                            .map_or(json!(false), ProductionClock::historical_advancement),
                        leaf.inventory_clock.as_ref().map_or(
                            json!(false),
                            ProductionInventoryClock::historical_advancement,
                        ),
                    ]
                })
                .collect()
        })
    }
    pub(super) fn portfolio_floors(&self) -> u64 {
        self.portfolio.as_ref().map_or(0, |p| {
            p.leaves
                .values()
                .map(|leaf| {
                    leaf.monitor
                        .as_ref()
                        .map_or(0, |m| m.high_tick().get())
                        .max(leaf.clock.as_ref().map_or(0, ProductionClock::high_tick))
                        .max(
                            leaf.inventory_clock
                                .as_ref()
                                .map_or(0, ProductionInventoryClock::high_tick),
                        )
                })
                .max()
                .unwrap_or(0)
        })
    }
    pub(super) fn portfolio_fenced(&self) -> bool {
        self.portfolio.as_ref().is_some_and(|p| {
            p.leaves.values().any(|leaf| {
                leaf.monitor
                    .as_ref()
                    .is_some_and(|m| m.parent_observation_fenced())
            })
        })
    }
    pub(super) fn portfolio_recovery(&self) -> Option<Value> {
        self.portfolio.as_ref().map(|p| Value::Array(self.portfolio_ids().into_iter().map(|step| {
            if step == p.active {
                json!({"step_id":step.get(),"goal_history_present":self.goal_history_present,
                    "clock_journal_id":self.clock.as_ref().and_then(ProductionClock::journal_id).map(|d|d.to_hex()),
                    "inventory_clock_journal_id":self.inventory_clock.as_ref().and_then(ProductionInventoryClock::journal_id).map(|d|d.to_hex())})
            } else { p.leaves.get(&step).map_or(Value::Null, |leaf| json!({"step_id":step.get(),"goal_history_present":leaf.history_present,
                "clock_journal_id":leaf.clock.as_ref().and_then(ProductionClock::journal_id).map(|d|d.to_hex()),
                "inventory_clock_journal_id":leaf.inventory_clock.as_ref().and_then(ProductionInventoryClock::journal_id).map(|d|d.to_hex())})) }
        }).collect()))
    }
    pub(super) fn portfolio_json(&self) -> Value {
        let Some(p) = &self.portfolio else {
            return Value::Null;
        };
        let goals = self.portfolio_ids().into_iter().map(|step| {
            if step == p.active { return json!({"step_id":step.get(), "goal":self.selected_goal_json(),
                "clock":self.clock_json(),"inventory_clock":self.inventory_clock_json()}); }
            p.leaves.get(&step).map_or(Value::Null, |leaf| {
                let mut goal = leaf.goal_json.clone();
                if leaf.current_withdrawn || leaf.monitor.as_ref().is_none_or(|m| m.latest().is_none()) {
                    goal["current_truth"] = Value::Null; goal["proven_at_anchor"] = json!(false); goal["anchor"] = Value::Null;
                }
                json!({"step_id":step.get(),"goal":goal,"clock":leaf.clock.as_ref().map(ProductionClock::to_json),
                    "inventory_clock":leaf.inventory_clock.as_ref().map(ProductionInventoryClock::to_json)})
            })
        }).collect::<Vec<_>>();
        json!({"schema":"dfmcp.production-portfolio/1","selected_order_step_id":p.active.get(),"orders":goals,
            "sampled_orders":p.cohort.as_ref().map_or(0, |(_,samples)|samples.len()),
            "next_observation":"fortress.wait","sampling":"one_original_order_per_bounded_wait",
            "complete_original_goal_proven":self.portfolio_proven()})
    }
    pub(super) fn portfolio_proven(&self) -> bool {
        self.portfolio
            .as_ref()
            .and_then(|p| p.current.as_ref())
            .is_some_and(|p| {
                p.anchor == self.anchor && p.original_goal_proven && !self.clock_withholds_goal()
            })
    }
    pub(super) fn portfolio_parent_json(&self) -> Value {
        self.portfolio.as_ref().and_then(|p|p.current.as_ref()).filter(|p|p.anchor == self.anchor && !self.clock_withholds_goal())
            .map_or(Value::Null, |p|json!({"scope":"complete_original_production_portfolio","current_truth":goals::truth(p.current_truth),
                "original_goal_proven":self.portfolio_proven(),"anchor":anchor_json(p.anchor),
                "selected_anchors":p.selected_anchors.iter().map(|(step,anchor)|json!({"step_id":step.get(),"anchor":anchor_json(*anchor)})).collect::<Vec<_>>(),
                "source_digest":p.source_digest.to_hex(),"workforce_source_digest":p.workforce_source_digest.to_hex(),
                "labor":p.labor.iter().map(|f|json!({"unit_id":f.unit_id.get().to_string(),"labor":f.labor,"requested_enabled":f.requested_enabled,
                    "observed_enabled":f.observed_enabled,"current_truth":goals::truth(f.current_truth)})).collect::<Vec<_>>(),
                "inventory_requirements":goals::inventory_requirements_json(&p.inventory_requirements),
                "parent_stability":self.portfolio.as_ref().and_then(|portfolio|portfolio.parent_stability.as_ref())
                    .map(|stability|stability.to_json(p.current_truth == PredicateTruth::True)),
                "restored_from_history":false,"production_attribution":"unknown"}))
    }
    fn portfolio_capture(
        &mut self,
        ids: &[u32],
        c: &OperationContext,
    ) -> Result<ProgressObservation> {
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        self.config.current()?;
        let mut source = self.config.orders.progress(c)?;
        let capture = source.read(ids, Duration::from_millis(c.budget.max_wall_millis))?;
        self.observation_tick_floor = self.observation_tick_floor.max(GameTick(capture.tick()));
        if capture.generation() != source.manifest().generation
            || capture.fortress_id() != self.anchor.fortress_id
            || !capture.paused()
            || capture.tick() < self.anchor.tick.get()
        {
            source.fence();
            return Err(denied(
                "portfolio progress bracket changed its original paused source",
            ));
        }
        self.config.current()?;
        Ok(capture)
    }
    pub(super) fn wait_portfolio(&mut self, c: &OperationContext) -> Result<View> {
        let result = self.wait_portfolio_inner(c);
        self.raise_goal_floor();
        self.raise_clock_floor();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    fn wait_portfolio_inner(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        if self.mode == Mode::Offline {
            return self.inspect(c);
        }
        if let Some(p) = &mut self.portfolio {
            p.current = None;
        }
        let mut work = Work::new(c)?;
        let output = work.take(256 * MIB)?;
        let ready = self
            .frontier
            .as_mut()
            .ok_or_else(|| denied("portfolio frontier absent"))?
            .inspect(&work.take(8 * MIB)?)?;
        if !ready.all_applied() || ready.needs_reconciliation() {
            self.clear_portfolio_cohort();
            return self.wait_with_goal(&work.rest(MAX_BYTES)?);
        }
        let active = self
            .selected_order_step()
            .ok_or_else(|| denied("selected portfolio order absent"))?;
        for step in self.portfolio_ids() {
            self.activate_leaf(step)?;
            self.reconcile_clock(&mut work)?;
            self.reconcile_inventory_clock(&mut work)?;
        }
        self.activate_leaf(active)?;
        if self.inactive_clock_may_be_running()
            || self
                .clock
                .as_ref()
                .is_some_and(ProductionClock::may_be_running)
            || self
                .inventory_clock
                .as_ref()
                .is_some_and(ProductionInventoryClock::may_be_running)
        {
            self.clear_portfolio_cohort();
            return self.view(&mut Work::new(&work.narrow(&output)?)?);
        }
        // Replay/initialize one missing history per bounded call. A retained
        // history may consume the admitted replay allowance; no sample is then
        // silently paid for by a renewed budget.
        let mut missing = Vec::new();
        for step in self.portfolio_ids() {
            self.activate_leaf(step)?;
            if self.goal_monitor.is_none() {
                missing.push(step);
            }
        }
        self.activate_leaf(active)?;
        if let Some(step) = missing.first() {
            self.activate_leaf(*step)?;
            let replay = self.goal_history_present;
            self.start_goal_monitor(!replay, &mut work)?;
            self.raise_goal_floor();
            work.context.anchor = self.anchor;
            if replay || missing.len() > 1 {
                return self.view(&mut Work::new(&work.narrow(&output)?)?);
            }
        }
        let mut native_ids = Vec::new();
        for step in self.portfolio_ids() {
            self.activate_leaf(step)?;
            native_ids.push(
                self.goal_monitor
                    .as_ref()
                    .ok_or_else(|| denied("portfolio history absent"))?
                    .native_order_id(),
            );
        }
        native_ids.sort_unstable();
        if self.portfolio.as_ref().is_some_and(|p| p.cohort.is_none()) {
            let before = self.portfolio_capture(&native_ids, &work.take(16 * MIB)?)?;
            self.portfolio
                .as_mut()
                .ok_or_else(|| denied("portfolio absent"))?
                .cohort = Some((before, Vec::new()));
        }
        let next = self
            .portfolio
            .as_ref()
            .and_then(|p| p.cohort.as_ref())
            .map_or(0, |(_, s)| s.len());
        let step = *self
            .portfolio_ids()
            .get(next)
            .ok_or_else(|| denied("portfolio cohort exhausted"))?;
        self.activate_leaf(step)?;
        let final_capture = work.take(16 * MIB)?;
        let final_proof = work.take(128 * MIB)?;
        let mut sample_context = work.rest(MAX_BYTES)?;
        sample_context.anchor = self.anchor;
        self.wait_with_goal_inner(&sample_context)?;
        let sample = self
            .portfolio_sample
            .take()
            .ok_or_else(|| denied("portfolio leaf did not publish a fresh joined sample"))?;
        let p = self
            .portfolio
            .as_mut()
            .ok_or_else(|| denied("portfolio absent"))?;
        let (_, samples) = p
            .cohort
            .as_mut()
            .ok_or_else(|| denied("portfolio bracket absent"))?;
        samples.push(sample);
        if samples.len() == native_ids.len() {
            work.context.anchor = self.anchor;
            let after = self.portfolio_capture(&native_ids, &work.narrow(&final_capture)?)?;
            let (before, samples) = self
                .portfolio
                .as_mut()
                .and_then(|p| p.cohort.take())
                .ok_or_else(|| denied("portfolio bracket absent"))?;
            let original = self
                .original
                .as_ref()
                .ok_or_else(|| denied("original portfolio absent"))?;
            let current = work.narrow(&final_proof)?;
            self.config.current()?;
            let mut result = if self.layout.as_ref().is_some_and(|layout| layout.existing_labor().is_some()) {
                live_production::compose_existing_labor_portfolio_goal(
                    original, &samples, &before, &after, &current,
                    |step, projection, context| {
                        self.config.current()?;
                        self.config.orders.acceptance().goal_for_selected(original, step, projection, context)
                    },
                    |projection, context| {
                        self.config.current()?;
                        self.config.workforce.acceptance().existing_labor(projection, context)
                    },
                )?
            } else { live_production::compose_production_portfolio_goal(
                original,
                &samples,
                &before,
                &after,
                &current,
                |step, projection, context| {
                    self.config.current()?;
                    self.config
                        .orders
                        .acceptance()
                        .goal_for_selected(original, step, projection, context)
                },
                |projection, context| {
                    self.config.current()?;
                    self.config.workforce.acceptance().goal(projection, context)
                },
            )? };
            let portfolio = self.portfolio.as_mut()
                .ok_or_else(|| denied("portfolio absent"))?;
            if let Some(stability) = &mut portfolio.parent_stability {
                // Count only complete qualified cohorts. The conjunction keeps
                // each selected original temporal requirement mandatory too.
                result.original_goal_proven &=
                    stability.observe(result.anchor, result.current_truth)?;
            }
            self.anchor = result.anchor;
            portfolio.current = Some(result);
        }
        work.context.anchor = self.anchor;
        self.view(&mut Work::new(&work.narrow(&output)?)?)
    }
}

#[cfg(test)]
mod stability_tests {
    include!("portfolio_stability_tests.rs");
}
