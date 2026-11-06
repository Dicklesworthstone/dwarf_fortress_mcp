//! Separately confirmed run/1.13 advancement for an already true original
//! inventory objective whose stability window still needs later observations.
//! This owner never supplies missing production attribution or order evidence.
use super::*;
use dfmcp_adapter::bounded_run::journal::RunState;
use dfmcp_adapter::bounded_run::rpc::RunSource;

fn viable(goal: &WorkOrderGoalProgress) -> Result<()> {
    if goal.current_goal != PredicateTruth::True
        || goal.failure_predicate == Some(PredicateTruth::True)
        || !matches!(
            goal.obligation,
            Some(ObligationStatus::Pending | ObligationStatus::Active { .. })
        )
        || goal.current_goal_proven()
    {
        return Err(denied(
            "inventory advancement requires qualified true inventory and unfinished original stability",
        ));
    }
    Ok(())
}

impl<S: Shell> Runtime<S> {
    pub fn inventory_clock_control_enabled(&self) -> bool {
        self.mode == Mode::Control
            && self.shell.inventory_clock_enabled()
            && self
                .inventory_clock
                .as_ref()
                .is_some_and(InventoryClock::is_control)
    }
    pub(super) fn inventory_clock_target(
        &self,
        creation: Option<&NativeOrderCreation>,
    ) -> Result<Option<InventoryClockTarget>> {
        let Some(creation @ NativeOrderCreation::Inserted { .. }) = creation else {
            return Ok(None);
        };
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| denied("original inventory goal absent"))?;
        // An empty optional owner may coexist with a legacy order objective.
        // Existing run history without this exact objective is rejected by inspect.
        if ObservedInventoryObjective::from_plan(original).is_none() {
            return Ok(None);
        }
        InventoryClockTarget::new(
            original,
            self.archive
                .as_ref()
                .ok_or_else(|| denied("original archive absent"))?
                .seal(),
            self.journal,
            self.review
                .as_ref()
                .ok_or_else(|| denied("original review absent"))?,
            creation,
        )
        .map(Some)
    }
    pub(super) fn inspect_inventory_clock(
        &mut self,
        c: &OperationContext,
    ) -> Result<Option<InventoryClockView>> {
        if self.inventory_clock.is_none() {
            return Ok(None);
        }
        let mut work = Work::new(c)?;
        let retained = self
            .review
            .as_ref()
            .map(|review| {
                self.semantic
                    .inspect(review.seal(), &work.reserve(16 * MIB)?)
            })
            .transpose()?;
        let target =
            self.inventory_clock_target(retained.as_ref().map(|value| value.creation_result()))?;
        let canonical = work.reserve(8 * MIB)?;
        let native = self.shell.inventory_clock_context(&canonical, false)?;
        self.inventory_clock
            .as_mut()
            .ok_or_else(|| denied("inventory clock owner absent"))?
            .inspect(target.as_ref(), &canonical, &native)
            .map(Some)
    }
    pub(super) fn reconcile_inventory_clock(
        &mut self,
        creation: &NativeOrderCreation,
        work: &mut Work,
    ) -> Result<bool> {
        if self.inventory_clock.is_none() {
            return Ok(true);
        }
        let target = self.inventory_clock_target(Some(creation))?;
        let canonical = work.reserve(8 * MIB)?;
        let native = self.shell.inventory_clock_context(&canonical, false)?;
        let clock = self
            .inventory_clock
            .as_mut()
            .ok_or_else(|| denied("inventory clock owner absent"))?;
        let current = clock.inspect(target.as_ref(), &canonical, &native)?;
        if !current.needs_reconciliation() {
            return Ok(current.can_sample_goal());
        }
        let target = target.as_ref().ok_or_else(|| {
            denied("inventory clock history lacks its original verified insertion")
        })?;
        let connect = self
            .shell
            .inventory_clock_context(&work.reserve(4 * MIB)?, false)?;
        let canonical = work.reserve(96 * MIB)?;
        let native = self.shell.inventory_clock_context(&canonical, false)?;
        let mut source = self.shell.inventory_clock_source(&connect)?;
        Ok(clock
            .reconcile(target, &mut source, &canonical, &native)?
            .can_sample_goal())
    }
    fn check_inventory_clock(&self, c: &OperationContext) -> Result<()> {
        self.check(c, false)?;
        if !self.inventory_clock_control_enabled() {
            return Err(denied(
                "this owner has no independent inventory clock authority",
            ));
        }
        c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
    }
    // Eight complete poll publications each verify their retained prefix twice;
    // reserve that history cost before contacting the independent clock source.
    fn inventory_sample_bytes(&self, base: u64) -> Result<u64> {
        (self
            .monitor
            .as_ref()
            .map_or(0, |monitor| monitor.history_byte_len()) as u64)
            .checked_mul(49)
            .and_then(|bytes| bytes.checked_add(base))
            .ok_or_else(|| exhausted("inventory clock evidence allowance overflow"))
    }
    fn sample_for_inventory_clock(&mut self, work: &mut Work) -> Result<InventoryClockTarget> {
        let sample = self.wait(&work.reserve(self.inventory_sample_bytes(768 * MIB)?)?)?;
        work.context.anchor = self.anchor;
        if sample
            .clock
            .as_ref()
            .is_some_and(|clock| clock.pending || clock.unresolved)
        {
            return Err(denied(
                "settle or cancel the independent order clock before an inventory run",
            ));
        }
        if sample
            .inventory_clock
            .as_ref()
            .is_some_and(|clock| !clock.can_sample_goal())
        {
            return Err(denied(
                "reconcile the retained inventory clock before another run",
            ));
        }
        let goal = sample
            .goal
            .as_ref()
            .filter(|goal| goal.anchor == self.anchor)
            .ok_or_else(|| {
                denied("inventory advancement requires a fresh qualified inventory sample")
            })?;
        viable(goal)?;
        self.inventory_clock_target(sample.creation.as_ref())?
            .ok_or_else(|| denied("inventory advancement requires the exact original inventory objective and Created receipt"))
    }
    /// Runs after the durable clock marker, before native Prepare/Commit. The
    /// source incarnation alone cannot establish named-fortress inventory truth.
    #[allow(clippy::too_many_arguments)]
    fn verify_inventory_clock_edge(
        shell: &S,
        semantic: &mut Semantic<S>,
        archive: &mut PlanArchiveStore<S::Archive>,
        monitor: &mut DurableWorkOrderGoalMonitor<S::History>,
        order_clock: Option<&mut Clock<S::ClockJournal>>,
        original: &PreparedPlan,
        review: &SemanticWorkOrderReview,
        target: &InventoryClockTarget,
        journal: Digest32,
        c: &OperationContext,
    ) -> Result<()> {
        shell.check_environment()?;
        if !shell.inventory_clock_enabled() {
            return Err(denied("independent inventory clock authority was revoked"));
        }
        let mut work = Work::new(c)?;
        if archive.recover(&work.reserve(ARCHIVE_BYTES)?)? != *original {
            return Err(denied(
                "original inventory archive changed before native clock effect",
            ));
        }
        let retained = semantic.inspect(review.seal(), &work.reserve(16 * MIB)?)?;
        let current = InventoryClockTarget::new(
            original,
            archive.seal(),
            journal,
            review,
            retained.creation_result(),
        )?;
        if current.digest() != target.digest() {
            return Err(denied("original inventory insertion identity changed"));
        }
        if let Some(clock) = order_clock {
            let original_target = ClockTarget::new(
                original,
                archive.seal(),
                journal,
                review,
                retained.creation_result(),
            )?;
            let order = clock.inspect(Some(&original_target), &work.reserve(8 * MIB)?)?;
            if order.pending || order.unresolved {
                return Err(denied(
                    "independent order clock became unsettled before inventory advancement",
                ));
            }
        }
        let cursor = ObservationCursor {
            epoch: c.anchor.cursor.epoch,
            sequence: c
                .anchor
                .cursor
                .sequence
                .max(monitor.high_cursor().sequence)
                .checked_add(1)
                .ok_or_else(|| exhausted("inventory goal cursor exhausted"))?,
        };
        let mut archive_work = Work::new(&work.reserve(4 * ARCHIVE_BYTES + 1)?)?;
        let final_archive = work.reserve(ARCHIVE_BYTES)?;
        let acceptance = shell.acceptance().clone();
        let sample = semantic.poll_original_goal_durable(
            monitor,
            cursor,
            &work.rest()?,
            |current| shell.progress(current),
            |current| shell.operations(current),
            |projection, current| {
                shell.check_environment()?;
                if !shell.inventory_clock_enabled() || projection.anchor().tick != c.anchor.tick {
                    return Err(denied(
                        "inventory source or independent clock authority changed after review",
                    ));
                }
                let mut before = archive_work.reserve(ARCHIVE_BYTES)?;
                before.budget.max_wall_millis = before
                    .budget
                    .max_wall_millis
                    .min(current.budget.max_wall_millis);
                if archive.recover(&before)? != *original {
                    return Err(denied(
                        "original inventory archive changed before clock authorization",
                    ));
                }
                let policy = acceptance.goal_for(original, projection, current)?;
                let mut after = archive_work.reserve(ARCHIVE_BYTES)?;
                after.budget.max_wall_millis = after
                    .budget
                    .max_wall_millis
                    .min(current.budget.max_wall_millis);
                if archive.recover(&after)? != *original {
                    return Err(denied(
                        "original inventory archive changed during clock authorization",
                    ));
                }
                shell.check_environment()?;
                Ok(policy)
            },
        )?;
        let goal = sample.progress();
        if goal.anchor.tick != c.anchor.tick {
            return Err(denied("inventory evidence changed after clock review"));
        }
        viable(goal)?;
        work.context.anchor = goal.anchor;
        if archive.recover(&work.narrow(&final_archive)?)? != *original {
            return Err(denied(
                "original inventory archive changed at the native clock edge",
            ));
        }
        shell.check_environment()?;
        if !shell.inventory_clock_enabled() {
            return Err(denied(
                "independent inventory clock authority was revoked at the native edge",
            ));
        }
        work.current()?;
        Ok(())
    }
    pub fn plan_inventory_run(
        &mut self,
        request: InventoryRunRequest,
        c: &OperationContext,
    ) -> Result<View> {
        self.check_inventory_clock(c)?;
        self.inventory_run(Some(request), None, c)
    }
    pub fn commit_inventory_run(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
    ) -> Result<View> {
        self.check_inventory_clock(c)?;
        if !confirmed {
            return Err(denied(
                "confirm the exact original-bound inventory clock review",
            ));
        }
        self.inventory_run(None, Some(seal), c)
    }
    fn inventory_run(
        &mut self,
        request: Option<InventoryRunRequest>,
        seal: Option<Digest32>,
        c: &OperationContext,
    ) -> Result<View> {
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_archive(&mut work)?;
        if let Some(seal) = seal {
            let current = self
                .inspect_inventory_clock(&work.reserve(32 * MIB)?)?
                .ok_or_else(|| denied("inventory clock not configured"))?;
            let selected = current
                .runs
                .iter()
                .find(|run| run.review_seal == seal)
                .ok_or_else(|| denied("inventory clock commit requires its exact review seal"))?;
            if selected.entry.terminal() && !selected.entry.unresolved() {
                return self.view(&work.narrow(&output)?);
            }
            if selected.entry.state() != RunState::Prepared {
                return Err(DfmcpError::new(
                    ErrorCode::EffectIndeterminate,
                    "reconcile the original inventory run; never repeat unpause",
                ));
            }
        }
        let target = self.sample_for_inventory_clock(&mut work)?;
        // The post-marker sample has separate proof authority; optional item
        // presentation must be acquired again by the next foreground wait.
        self.item_observations = None;
        let mut guard_budget = Some(work.reserve(self.inventory_sample_bytes(640 * MIB)?)?);
        let canonical = work.reserve(96 * MIB)?;
        let connect = self
            .shell
            .inventory_clock_context(&work.reserve(4 * MIB)?, true)?;
        let native = self.shell.inventory_clock_context(&canonical, true)?;
        let mut source = self.shell.inventory_clock_source(&connect)?;
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| denied("original inventory goal absent"))?;
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| denied("original review absent"))?;
        let archive = self
            .archive
            .as_mut()
            .ok_or_else(|| denied("original archive absent"))?;
        let monitor = self
            .monitor
            .as_mut()
            .ok_or_else(|| denied("original inventory monitor absent"))?;
        let semantic = &mut self.semantic;
        let mut order_clock = self.clock.as_mut();
        let shell = &self.shell;
        let journal = self.journal;
        let guard = |edge: &OperationContext| {
            let mut reserved = guard_budget
                .take()
                .ok_or_else(|| denied("inventory clock guard cannot repeat"))?;
            reserved.anchor = edge.anchor;
            reserved.budget.max_wall_millis = reserved
                .budget
                .max_wall_millis
                .min(edge.budget.max_wall_millis);
            Self::verify_inventory_clock_edge(
                shell,
                semantic,
                archive,
                monitor,
                order_clock.as_deref_mut(),
                original,
                review,
                &target,
                journal,
                &reserved,
            )
        };
        let clock = self
            .inventory_clock
            .as_mut()
            .ok_or_else(|| denied("inventory clock source not configured"))?;
        let result = if let Some(request) = request {
            clock.prepare_guarded(
                &target,
                request.inner(),
                &mut source,
                &canonical,
                &native,
                guard,
            )
        } else {
            self.inventory_clock_invalidates_goal = true;
            self.basis = None;
            self.evidence = None;
            self.item_observations = None;
            clock.commit_guarded(
                &target,
                seal.ok_or_else(|| invalid("inventory review absent"))?,
                true,
                &mut source,
                &canonical,
                &native,
                guard,
            )
        };
        self.raise_monitor_floor();
        work.context.anchor = self.anchor;
        self.opening_inventory_clock = Some(result?);
        self.view(&work.narrow(&output)?)
    }
    pub fn cancel_inventory_run(&mut self, c: &OperationContext) -> Result<View> {
        // Safety pause stays available after deadline or loss of Observe/Plan.
        self.check(c, false)?;
        if !self.inventory_clock_control_enabled() {
            return Err(denied(
                "this owner has no independent inventory clock authority",
            ));
        }
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_archive(&mut work)?;
        let current = self
            .inspect_inventory_clock(&work.reserve(32 * MIB)?)?
            .ok_or_else(|| denied("inventory clock not configured"))?;
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| denied("original review absent"))?;
        let retained = self
            .semantic
            .inspect(review.seal(), &work.reserve(16 * MIB)?)?;
        let target = self
            .inventory_clock_target(Some(retained.creation_result()))?
            .ok_or_else(|| {
                denied("inventory clock cancellation requires its original insertion identity")
            })?;
        let mut source = if current.cancellation_needs_source() {
            let connect = self
                .shell
                .inventory_clock_context(&work.reserve(4 * MIB)?, true)?;
            Some(self.shell.inventory_clock_source(&connect)?)
        } else {
            None
        };
        let canonical = work.reserve(96 * MIB)?;
        let native = self.shell.inventory_clock_context(&canonical, true)?;
        self.inventory_clock_invalidates_goal |= current.has_runs();
        self.opening_inventory_clock = Some(
            self.inventory_clock
                .as_mut()
                .ok_or_else(|| denied("inventory clock owner absent"))?
                .cancel(
                    &target,
                    source.as_mut().map(|source| source as &mut dyn RunSource),
                    &canonical,
                    &native,
                )?,
        );
        self.view(&work.narrow(&output)?)
    }
}
