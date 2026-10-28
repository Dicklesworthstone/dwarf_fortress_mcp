//! Separately confirmed finite advancement retains the original construction
//! receipt, plan, deadline and durable completion/stability proof.
use super::*;

fn viable(progress: &FurnitureGoalProgress) -> Result<()> {
    if !progress.receipt_linked
        || !matches!(progress.construction_condition, "pending" | "condition_met")
        || progress.current_goal == PredicateTruth::Unknown
        || progress.failure_predicate == Some(PredicateTruth::True)
        || !matches!(
            progress.obligation,
            Some(ObligationStatus::Pending | ObligationStatus::Active { .. })
        )
        || progress.current_goal_proven()
    {
        return Err(denied(
            "clock advancement requires a viable unfinished original construction goal",
        ));
    }
    Ok(())
}

impl<S: Shell> Runtime<S> {
    pub fn clock_control_enabled(&self) -> bool {
        self.mode == Mode::Control
            && self.shell.clock_enabled()
            && self.clock.as_ref().is_some_and(Clock::is_control)
    }
    pub(super) fn clock_target(
        &self,
        placement: Option<&BuildEntry>,
    ) -> Result<Option<ClockTarget>> {
        let Some(placement) = placement.filter(|entry| {
            entry.state() == BuildState::Terminal
                && entry
                    .native()
                    .is_some_and(|record| record.phase() == BuildPhase::Placed)
        }) else {
            return Ok(None);
        };
        ClockTarget::new(
            self.original
                .as_ref()
                .ok_or_else(|| denied("original construction goal absent"))?,
            self.archive
                .as_ref()
                .ok_or_else(|| denied("original plan archive absent"))?
                .seal(),
            self.journal,
            self.review
                .as_ref()
                .ok_or_else(|| denied("original paired review absent"))?,
            placement,
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
        let target = self.clock_target(result.as_ref().map(|result| result.native_record()))?;
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
    /// The durable monitor re-verifies each complete retained prefix before its
    /// next append. Pay that bounded work before connecting a native clock.
    fn clock_sample_bytes(&self, base: u64) -> Result<u64> {
        (self
            .monitor
            .as_ref()
            .map_or(0, |monitor| monitor.history_byte_len()) as u64)
            .checked_mul(25)
            .and_then(|bytes| bytes.checked_add(base))
            .ok_or_else(|| exhausted("clock construction evidence allowance overflow"))
    }
    fn sample_for_clock(&mut self, work: &mut Work) -> Result<ClockTarget> {
        let bytes = self.clock_sample_bytes(768 * MIB)?;
        let sample = self.wait(&work.reserve(bytes)?)?;
        work.context.anchor = self.authority;
        if sample
            .clock
            .as_ref()
            .is_some_and(|clock| !clock.can_sample_goal())
        {
            return Err(denied(
                "reconcile the retained clock before another construction run",
            ));
        }
        let goal = sample
            .goal
            .as_ref()
            .filter(|goal| Some(goal.anchor) == self.anchor)
            .ok_or_else(|| denied("clock control requires fresh original construction evidence"))?;
        viable(goal)?;
        self.clock_target(sample.native.as_ref())?
            .ok_or_else(|| denied("clock advancement requires the verified original placement"))
    }
    /// Runs after the durable clock marker, immediately before native PrepareRun
    /// or CommitRun. The source clock has no named-fortress identity: independently
    /// qualified receipt-linked construction evidence must still be current.
    #[allow(clippy::too_many_arguments)]
    fn verify_clock_edge(
        shell: &S,
        semantic: &mut Semantic<S>,
        archive: &mut PlanArchiveStore<S::Archive>,
        monitor: &mut DurableFurnitureGoalMonitor<S::History>,
        original: &PreparedPlan,
        review: &SemanticFurnitureReview,
        target: &ClockTarget,
        journal: Digest32,
        authority: &mut StateAnchor,
        c: &OperationContext,
    ) -> Result<()> {
        shell.check_environment()?;
        if !shell.clock_enabled() {
            return Err(denied("operator clock authority was revoked"));
        }
        let mut work = Work::new(c)?;
        if archive.recover_at_authority_floor(&work.reserve(ARCHIVE_BYTES)?)? != *original {
            return Err(denied("original plan changed before native clock effect"));
        }
        let retained = semantic.inspect(review.seal(), &work.reserve(16 * MIB)?)?;
        let current = ClockTarget::new(
            original,
            archive.seal(),
            journal,
            review,
            retained.native_record(),
        )?;
        if current.digest() != target.digest() {
            return Err(denied(
                "original placement identity changed before native clock effect",
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
        let mut archive_work = Work::new(&work.reserve(4 * ARCHIVE_BYTES + 1)?)?;
        let final_archive = work.reserve(ARCHIVE_BYTES)?;
        let sample = semantic.poll_original_goal_durable(
            monitor,
            cursor,
            &work.rest()?,
            |binding, current| shell.goal_source(binding, current),
            |projection, current| {
                archive_work.boundary(current)?;
                Self::retain_authority_floor(authority, current.anchor)?;
                shell.check_environment()?;
                if !shell.clock_enabled() || projection.anchor().tick != c.anchor.tick {
                    return Err(denied(
                        "construction source or clock authority changed after review",
                    ));
                }
                if archive.recover_at_authority_floor(&archive_work.reserve(ARCHIVE_BYTES)?)?
                    != *original
                {
                    return Err(denied(
                        "original archive changed before native clock authorization",
                    ));
                }
                let policy = shell.goal_policy(projection, current)?;
                if archive.recover_at_authority_floor(&archive_work.reserve(ARCHIVE_BYTES)?)?
                    != *original
                {
                    return Err(denied(
                        "original archive changed during native clock authorization",
                    ));
                }
                shell.check_environment()?;
                Ok(policy)
            },
        )?;
        let progress = sample.progress();
        if progress.anchor.tick != c.anchor.tick {
            return Err(denied(
                "construction source changed after clock review; native unpause refused",
            ));
        }
        viable(progress)?;
        // Both policy callbacks already consumed this newer real canonical
        // cursor. Final custody verification must retain that publication; the
        // source clock's cursor cannot stand in for its named-fortress anchor.
        work.context.anchor = progress.anchor;
        if archive.recover_at_authority_floor(&work.narrow(&final_archive)?)? != *original {
            return Err(denied("original plan changed at the native clock edge"));
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
        let mut guard_budget = Some(work.reserve(self.clock_sample_bytes(640 * MIB)?)?);
        let canonical = work.reserve(96 * MIB)?;
        let connect = self.shell.clock_context(&work.reserve(4 * MIB)?, true)?;
        let native = self.shell.clock_context(&canonical, true)?;
        let mut source = self.shell.clock_source(&connect)?;
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
        let monitor = self
            .monitor
            .as_mut()
            .ok_or_else(|| denied("original goal monitor absent"))?;
        let semantic = &mut self.semantic;
        let shell = &self.shell;
        let journal = self.journal;
        let authority = &mut self.authority;
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
                        shell, semantic, archive, monitor, original, review, &target, journal,
                        authority, &reserved,
                    )
                },
            );
        self.raise_floor();
        work.context.anchor = self.authority;
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
        let mut guard_budget = Some(work.reserve(self.clock_sample_bytes(640 * MIB)?)?);
        let canonical = work.reserve(96 * MIB)?;
        let connect = self.shell.clock_context(&work.reserve(4 * MIB)?, true)?;
        let native = self.shell.clock_context(&canonical, true)?;
        let mut source = self.shell.clock_source(&connect)?;
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
        let monitor = self
            .monitor
            .as_mut()
            .ok_or_else(|| denied("original goal monitor absent"))?;
        let semantic = &mut self.semantic;
        let shell = &self.shell;
        let journal = self.journal;
        let authority = &mut self.authority;
        self.clock_invalidates_goal = true;
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
                        shell, semantic, archive, monitor, original, review, &target, journal,
                        authority, &reserved,
                    )
                },
            );
        self.raise_floor();
        work.context.anchor = self.authority;
        self.opening_clock = Some(result?);
        self.view(&work.narrow(&output)?)
    }
    pub fn cancel_run(&mut self, c: &OperationContext) -> Result<View> {
        // Safety pause requires current independent clock authority and retained
        // identity, even after expiry, source failure or loss of Observe/Plan.
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
            .clock_target(Some(retained.native_record()))?
            .ok_or_else(|| denied("clock cancellation requires original placement identity"))?;
        let mut source = if current.cancellation_needs_source() {
            let connect = self.shell.clock_context(&work.reserve(4 * MIB)?, true)?;
            Some(self.shell.clock_source(&connect)?)
        } else {
            None
        };
        let canonical = work.reserve(96 * MIB)?;
        let native = self.shell.clock_context(&canonical, true)?;
        let result = self
            .clock
            .as_mut()
            .ok_or_else(|| denied("clock owner absent"))?
            .cancel(
                &target,
                source.as_mut().map(|source| source as &mut dyn RunSource),
                &canonical,
                &native,
            );
        self.clock_invalidates_goal |= current.has_runs();
        self.raise_floor();
        work.context.anchor = self.authority;
        self.opening_clock = Some(result?);
        self.view(&work.narrow(&output)?)
    }
}
