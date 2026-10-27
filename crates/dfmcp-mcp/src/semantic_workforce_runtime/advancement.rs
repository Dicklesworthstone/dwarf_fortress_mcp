//! Explicit clock advancement retains the original labor goal and its proof.
//! Each run has independent consent, authority, custody and native source scope.
use super::*;

impl<S: Shell> Runtime<S> {
    pub fn clock_control_enabled(&self) -> bool {
        self.mode == Mode::Control
            && self.shell.clock_enabled()
            && self.clock.as_ref().is_some_and(Clock::is_control)
    }
    pub(super) fn clock_target(
        &self,
        action: Option<&SingleLaborResult>,
    ) -> Result<Option<ClockTarget>> {
        let Some(action @ SingleLaborResult::Verified { .. }) = action else {
            return Ok(None);
        };
        ClockTarget::new(
            self.original
                .as_ref()
                .ok_or_else(|| denied("original labor goal absent"))?,
            self.archive
                .as_ref()
                .ok_or_else(|| denied("original plan archive absent"))?
                .seal(),
            self.journal,
            self.identity
                .as_ref()
                .ok_or_else(|| denied("original citizen identity absent"))?
                .seal(),
            self.review
                .as_ref()
                .ok_or_else(|| denied("original paired review absent"))?,
            action,
        )
        .map(Some)
    }
    pub(super) fn inspect_clock(&mut self, c: &OperationContext) -> Result<Option<ClockView>> {
        if self.clock.is_none() {
            return Ok(None);
        }
        let mut work = Work::new(c)?;
        let result = if let Some(review) = &self.review {
            Some(
                self.semantic
                    .inspect(review.seal(), &work.reserve(16 * MIB)?)?,
            )
        } else {
            None
        };
        let target = self.clock_target(result.as_ref().map(|r| r.action_result()))?;
        let canonical = work.reserve(8 * MIB)?;
        let native = self.shell.clock_context(&canonical, false)?;
        self.clock
            .as_mut()
            .ok_or_else(|| denied("clock owner absent"))?
            .inspect(target.as_ref(), &canonical, &native)
            .map(Some)
    }
    fn check_clock(&self, c: &OperationContext) -> Result<()> {
        self.check(c, false)?;
        if !self.clock_control_enabled() {
            return Err(denied(
                "this owner has no current independent clock authority",
            ));
        }
        c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
    }
    fn sample_for_clock(&mut self, work: &mut Work) -> Result<ClockTarget> {
        let sample = self.wait(&work.reserve(1024 * MIB)?)?;
        work.context.anchor = self.anchor;
        if sample
            .clock
            .as_ref()
            .is_some_and(|clock| !clock.can_sample_goal())
        {
            return Err(denied(
                "reconcile the retained clock before another labor run",
            ));
        }
        let goal = sample
            .goal
            .as_ref()
            .filter(|g| g.anchor == self.anchor)
            .ok_or_else(|| denied("clock control requires fresh original labor evidence"))?;
        if goal.current_goal != PredicateTruth::True
            || goal.failure_predicate == Some(PredicateTruth::True)
        {
            return Err(denied(
                "clock advancement requires current true original labor predicates",
            ));
        }
        if goal.current_goal_proven() {
            return Err(denied("the original labor goal is already proved"));
        }
        self.clock_target(sample.action.as_ref())?
            .ok_or_else(|| denied("clock advancement requires verified original assignment"))
    }
    /// Runs after the durable native clock marker, immediately before the
    /// PrepareRun/CommitRun call. Source-clock evidence cannot replace this
    /// independently qualified named-fortress labor observation.
    #[allow(clippy::too_many_arguments)]
    fn verify_clock_edge(
        shell: &S,
        semantic: &mut Semantic<S>,
        archive: &mut PlanArchiveStore<S::Archive>,
        identity: &mut CapsuleStore<S::Identity>,
        monitor: &mut DurableWorkforceGoalMonitor<S::History>,
        original: &PreparedPlan,
        review: &SemanticWorkforceReview,
        target: &ClockTarget,
        journal: Digest32,
        c: &OperationContext,
    ) -> Result<()> {
        shell.check_environment()?;
        if !shell.clock_enabled() {
            return Err(denied("operator clock authority was revoked"));
        }
        let mut work = Work::new(c)?;
        if archive.recover(&work.reserve(ARCHIVE_BYTES)?)? != *original {
            return Err(denied("original plan changed before native clock effect"));
        }
        let capsule = identity.recover(&work.reserve(IDENTITY_BYTES)?)?;
        let projection = project_live_capsule(
            &capsule,
            original.anchor.fortress_id,
            original.anchor.cursor,
        )?;
        if projection.snapshot.anchor() != original.anchor {
            return Err(denied(
                "original citizen identity changed before native clock effect",
            ));
        }
        shell
            .acceptance()
            .historical_citizens(&capsule, &projection, &work.reserve(MIB)?)?;
        let retained = semantic.inspect(review.seal(), &work.reserve(16 * MIB)?)?;
        let current = ClockTarget::new(
            original,
            archive.seal(),
            journal,
            identity.seal(),
            review,
            retained.action_result(),
        )?;
        if current.digest() != target.digest() {
            return Err(denied(
                "original assignment identity changed before native clock effect",
            ));
        }
        let cursor = ObservationCursor {
            epoch: c.anchor.cursor.epoch,
            sequence: c
                .anchor
                .cursor
                .sequence
                .max(monitor.high_cursor().sequence)
                .checked_add(1)
                .ok_or_else(|| exhausted("goal cursor exhausted"))?,
        };
        let sample = semantic.poll_original_goal_durable(
            monitor,
            cursor,
            &work.rest()?,
            |binding, c| shell.native(binding, c),
            |projection, c| {
                shell.check_environment()?;
                shell.acceptance().goal(projection, c)
            },
        )?;
        let progress = sample.progress();
        if progress.anchor.tick != c.anchor.tick
            || progress.current_goal != PredicateTruth::True
            || progress.failure_predicate == Some(PredicateTruth::True)
        {
            return Err(denied(
                "labor source changed after clock review; native unpause refused",
            ));
        }
        shell.check_environment()?;
        if !shell.clock_enabled() {
            return Err(denied(
                "operator clock authority was revoked at the native edge",
            ));
        }
        work.current()?;
        Ok(())
    }
    pub fn prepare_run(&mut self, request: RunRequest, c: &OperationContext) -> Result<View> {
        self.check_clock(c)?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_original(&mut work)?;
        let target = self.sample_for_clock(&mut work)?;
        let connect = self.shell.clock_context(&work.reserve(4 * MIB)?, true)?;
        let mut source = self.shell.clock_source(&connect)?;
        let mut guard_budget = Some(work.reserve(256 * MIB)?);
        let canonical = work.reserve(96 * MIB)?;
        let native = self.shell.clock_context(&canonical, true)?;
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| denied("original goal absent"))?;
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| denied("original review absent"))?;
        let archive = self
            .archive
            .as_mut()
            .ok_or_else(|| denied("original archive absent"))?;
        let identity = self
            .identity
            .as_mut()
            .ok_or_else(|| denied("original identity absent"))?;
        let monitor = self
            .monitor
            .as_mut()
            .ok_or_else(|| denied("original goal monitor absent"))?;
        let semantic = &mut self.semantic;
        let shell = &self.shell;
        let journal = self.journal;
        let result = self
            .clock
            .as_mut()
            .ok_or_else(|| denied("clock source not configured"))?
            .prepare_guarded(
                &target,
                &request,
                &mut source,
                &canonical,
                &native,
                |edge| {
                    let mut reserved = guard_budget
                        .take()
                        .ok_or_else(|| denied("clock guard cannot repeat"))?;
                    reserved.anchor = edge.anchor;
                    reserved.budget.max_wall_millis = reserved
                        .budget
                        .max_wall_millis
                        .min(edge.budget.max_wall_millis);
                    Self::verify_clock_edge(
                        shell, semantic, archive, identity, monitor, original, review, &target,
                        journal, &reserved,
                    )
                },
            );
        self.raise_monitor_floor();
        work.context.anchor = self.anchor;
        self.opening_clock = Some(result?);
        self.view(&work.narrow(&output)?)
    }
    pub fn commit_run(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
    ) -> Result<View> {
        self.check_clock(c)?;
        if !confirmed {
            return Err(denied("confirm the exact original-bound clock review"));
        }
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_original(&mut work)?;
        // Completed duplicates are readback only. A dispatch marker or running
        // record refuses before fresh sampling, never becoming another unpause.
        let current = self
            .inspect_clock(&work.reserve(32 * MIB)?)?
            .ok_or_else(|| denied("clock source not configured"))?;
        let selected = current
            .runs
            .iter()
            .find(|run| run.review_seal == seal)
            .ok_or_else(|| denied("clock commit requires its exact review seal"))?;
        if selected.entry.terminal() && !selected.entry.unresolved() {
            return self.view(&work.narrow(&output)?);
        }
        if selected.entry.state() != dfmcp_adapter::bounded_run::journal::RunState::Prepared {
            return Err(DfmcpError::new(
                ErrorCode::EffectIndeterminate,
                "reconcile the original clock run; never repeat unpause",
            ));
        }
        let target = self.sample_for_clock(&mut work)?;
        let connect = self.shell.clock_context(&work.reserve(4 * MIB)?, true)?;
        let mut source = self.shell.clock_source(&connect)?;
        let mut guard_budget = Some(work.reserve(256 * MIB)?);
        let canonical = work.reserve(96 * MIB)?;
        let native = self.shell.clock_context(&canonical, true)?;
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| denied("original goal absent"))?;
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| denied("original review absent"))?;
        let archive = self
            .archive
            .as_mut()
            .ok_or_else(|| denied("original archive absent"))?;
        let identity = self
            .identity
            .as_mut()
            .ok_or_else(|| denied("original identity absent"))?;
        let monitor = self
            .monitor
            .as_mut()
            .ok_or_else(|| denied("original goal monitor absent"))?;
        let semantic = &mut self.semantic;
        let shell = &self.shell;
        let journal = self.journal;
        self.basis = None;
        self.evidence = None;
        let result = self
            .clock
            .as_mut()
            .ok_or_else(|| denied("clock source not configured"))?
            .commit_guarded(
                &target,
                seal,
                true,
                &mut source,
                &canonical,
                &native,
                |edge| {
                    let mut reserved = guard_budget
                        .take()
                        .ok_or_else(|| denied("clock guard cannot repeat"))?;
                    reserved.anchor = edge.anchor;
                    reserved.budget.max_wall_millis = reserved
                        .budget
                        .max_wall_millis
                        .min(edge.budget.max_wall_millis);
                    Self::verify_clock_edge(
                        shell, semantic, archive, identity, monitor, original, review, &target,
                        journal, &reserved,
                    )
                },
            );
        self.raise_monitor_floor();
        work.context.anchor = self.anchor;
        self.opening_clock = Some(result?);
        self.view(&work.narrow(&output)?)
    }
    pub fn cancel_run(&mut self, c: &OperationContext) -> Result<View> {
        // Safety pause requires current independent clock authority and exact
        // retained clock/assignment identity, not fresh labor truth or deadline.
        self.check(c, false)?;
        if !self.clock_control_enabled() {
            return Err(denied(
                "this owner has no current independent clock authority",
            ));
        }
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        let current = self
            .inspect_clock(&work.reserve(32 * MIB)?)?
            .ok_or_else(|| denied("clock source not configured"))?;
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| denied("original review absent"))?;
        let retained = self
            .semantic
            .inspect(review.seal(), &work.reserve(16 * MIB)?)?;
        let target = self
            .clock_target(Some(retained.action_result()))?
            .ok_or_else(|| denied("clock cancellation requires original assignment identity"))?;
        let mut source = if current.cancellation_needs_source() {
            let connect = self.shell.clock_context(&work.reserve(4 * MIB)?, true)?;
            Some(self.shell.clock_source(&connect)?)
        } else {
            None
        };
        let canonical = work.reserve(96 * MIB)?;
        let native = self.shell.clock_context(&canonical, true)?;
        let view = self
            .clock
            .as_mut()
            .ok_or_else(|| denied("clock owner absent"))?
            .cancel(
                &target,
                source.as_mut().map(|source| source as &mut dyn RunSource),
                &canonical,
                &native,
            )?;
        self.opening_clock = Some(view);
        self.view(&work.narrow(&output)?)
    }
}
