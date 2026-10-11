//! Foreground progression of already committed laboratory work.
//!
//! A caller owns the loop and publishes each returned slice before requesting
//! another. This is not a background scheduler or a native execution path.
//! Original plans, identities, deadlines and obligation runtimes are retained.

use std::collections::BTreeSet;
use std::time::Instant;

use dfmcp_adapter::{ActionReceipt, GameAdapter};
use dfmcp_core::{
    ActionId, Capability, CommitState, DfmcpError, ErrorCode, GameTick, OperationContext, Result,
    RiskTier, SessionId, StateAnchor,
};
use dfmcp_lab::MemoryAdapter;

/// A quiet or recovered-only world still produces intermediate observations.
pub(crate) const DEFAULT_SLICE_TICKS: u64 = 100;
const MAX_PROGRESS_SLICES: u64 = 4_096;
const MAX_ACTION_POLLS: u64 = 65_536;
const MAX_TRACKED_ACTIONS: usize = 1_024;

/// One atomic simulated frontier, ready for the owning session to publish.
pub(crate) struct ProgressSlice {
    pub(crate) receipts: Vec<ActionReceipt>,
    pub(crate) finished: bool,
}

/// The work budget and exact frontier of one explicit wait request.
pub(crate) struct ForegroundProgress {
    session_id: SessionId,
    expected_anchor: StateAnchor,
    requested_ticks: u64,
    advanced_ticks: u64,
    slices: u64,
    polls: u64,
    dispatches: u32,
    max_actions: u32,
    max_wall_millis: u64,
    started: Instant,
    tracked: Option<BTreeSet<ActionId>>,
    finished: bool,
}

fn refusal(code: ErrorCode, message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(code, message)
}

impl ForegroundProgress {
    /// Admit the requested total once. The current clock and Observe grants
    /// must cover it, including its final endpoint when already unpaused.
    pub(crate) fn new(
        adapter: &MemoryAdapter,
        requested_ticks: u64,
        context: &OperationContext,
    ) -> Result<Self> {
        if context.anchor != adapter.snapshot().anchor() {
            return Err(refusal(
                ErrorCode::StaleAnchor,
                "foreground wait anchor is stale",
            ));
        }
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        if requested_ticks > context.budget.max_game_ticks {
            return Err(refusal(
                ErrorCode::BudgetExceeded,
                "foreground wait exceeds its admitted total game-tick budget",
            ));
        }
        if requested_ticks > 0 {
            context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
            let end = context
                .anchor
                .tick
                .0
                .checked_add(requested_ticks)
                .ok_or_else(|| {
                    refusal(
                        ErrorCode::BudgetExceeded,
                        "foreground wait exceeds the game-time horizon",
                    )
                })?;
            if !adapter.snapshot().paused {
                let mut final_context = context.clone();
                final_context.anchor.tick = GameTick(end);
                final_context.authorize(
                    Capability::ControlClock,
                    RiskTier::Reversible,
                    &[],
                    None,
                )?;
                final_context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            }
        }
        Ok(Self {
            session_id: context.session_id,
            expected_anchor: context.anchor,
            requested_ticks,
            advanced_ticks: 0,
            slices: 0,
            polls: 0,
            dispatches: 0,
            max_actions: context.budget.max_actions,
            max_wall_millis: context.budget.max_wall_millis,
            started: Instant::now(),
            tracked: None,
            finished: false,
        })
    }

    pub(crate) fn advanced_game_ticks(&self) -> u64 {
        self.advanced_ticks
    }

    pub(crate) fn remaining_game_ticks(&self) -> u64 {
        self.requested_ticks - self.advanced_ticks
    }

    pub(crate) fn slices(&self) -> u64 {
        self.slices
    }

    pub(crate) fn polls(&self) -> u64 {
        self.polls
    }

    pub(crate) fn dispatched_actions(&self) -> u32 {
        self.dispatches
    }

    fn check_deadline(&self, context: &OperationContext) -> Result<()> {
        if self.started.elapsed().as_millis()
            >= u128::from(self.max_wall_millis.min(context.budget.max_wall_millis))
        {
            return Err(refusal(
                ErrorCode::BudgetExceeded,
                "foreground wait reached its cooperative wall-time budget; retain this frontier and explicitly wait again for the remaining ticks",
            ));
        }
        Ok(())
    }

    /// Advance no further than the earliest current cadence, deadline or owner
    /// ceiling, then poll the retained actions in their original commit order.
    /// The first slice polls without advancing, including a zero-tick request.
    ///
    /// `slice_ceiling_ticks` lets the owner include recovered obligations that
    /// have no process-local adapter handle. It carries no evidence or authority.
    /// After a successful slice, the owner must observe carried work, preserve
    /// original-goal history and publish durability before calling this again.
    /// Any error rolls back this entire slice; earlier returned slices remain.
    pub(crate) fn step(
        &mut self,
        adapter: &mut MemoryAdapter,
        tracked_actions: &[ActionId],
        context: &OperationContext,
        slice_ceiling_ticks: Option<u64>,
    ) -> Result<Option<ProgressSlice>> {
        if self.finished {
            return Ok(None);
        }
        if context.session_id != self.session_id
            || context.anchor != self.expected_anchor
            || adapter.snapshot().anchor() != self.expected_anchor
        {
            return Err(refusal(
                ErrorCode::StaleAnchor,
                "foreground wait must retain its owning session and exact published frontier",
            ));
        }
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        self.check_deadline(context)?;
        if self.requested_ticks > context.budget.max_game_ticks {
            return Err(refusal(
                ErrorCode::BudgetExceeded,
                "foreground wait's total exceeds the current narrowed game-tick budget",
            ));
        }
        if slice_ceiling_ticks == Some(0) {
            return Err(refusal(
                ErrorCode::InvalidRequest,
                "foreground wait observation ceiling must be positive",
            ));
        }
        if tracked_actions.len() > MAX_TRACKED_ACTIONS
            || tracked_actions.len() > context.budget.max_entities as usize
        {
            return Err(refusal(
                ErrorCode::BudgetExceeded,
                "foreground wait action domain exceeds its bound",
            ));
        }
        let selected: BTreeSet<_> = tracked_actions.iter().copied().collect();
        if selected.len() != tracked_actions.len()
            || self
                .tracked
                .as_ref()
                .is_some_and(|initial| !selected.is_subset(initial))
        {
            return Err(refusal(
                ErrorCode::InvalidRequest,
                "foreground wait cannot duplicate or add action identities after admission",
            ));
        }
        let polls = self
            .polls
            .checked_add(tracked_actions.len() as u64)
            .ok_or_else(|| {
                refusal(
                    ErrorCode::BudgetExceeded,
                    "foreground wait action-poll counter exhausted",
                )
            })?;
        if self.slices >= MAX_PROGRESS_SLICES || polls > MAX_ACTION_POLLS {
            return Err(refusal(
                ErrorCode::BudgetExceeded,
                "foreground wait reached its observation-work bound; retain this frontier and explicitly wait again for the remaining ticks",
            ));
        }

        let mut interval = DEFAULT_SLICE_TICKS
            .min(slice_ceiling_ticks.unwrap_or(DEFAULT_SLICE_TICKS))
            .min(self.remaining_game_ticks());
        let now = adapter.snapshot().tick.0;
        // Validate every identity before any clock or deferred effect. The
        // owner supplies commit order, with prerequisites before dependents.
        let mut preceding = BTreeSet::new();
        for action_id in tracked_actions {
            let receipt = adapter.action_receipt(*action_id).ok_or_else(|| {
                refusal(
                    ErrorCode::InvalidRequest,
                    format!("unknown foreground action {action_id}"),
                )
            })?;
            let step = adapter.action_step(*action_id).ok_or_else(|| {
                refusal(
                    ErrorCode::InvalidPlan,
                    "foreground action has no original sealed step",
                )
            })?;
            let plan = adapter.action_plan_receipt(*action_id).ok_or_else(|| {
                refusal(
                    ErrorCode::InvalidPlan,
                    "foreground action has no original commit",
                )
            })?;
            for dependency in &step.depends_on {
                if let Some(parent) = adapter.step_receipt(plan.plan_id, *dependency)
                    && selected.contains(&parent.action_id)
                    && !preceding.contains(&parent.action_id)
                {
                    return Err(refusal(
                        ErrorCode::InvalidRequest,
                        "foreground actions must retain prerequisite-before-dependent order",
                    ));
                }
            }
            preceding.insert(*action_id);
            if matches!(
                receipt.state,
                CommitState::Prepared | CommitState::AppliedAwaitingVerification
            ) && let Some(obligation) = &step.obligation
            {
                if obligation.poll_interval_ticks == 0 {
                    return Err(refusal(
                        ErrorCode::InvalidPlan,
                        "foreground obligation has zero cadence",
                    ));
                }
                interval = interval.min(obligation.poll_interval_ticks);
                if obligation.deadline_tick.0 > now {
                    interval = interval.min(obligation.deadline_tick.0 - now);
                }
            }
        }
        let advance = if self.slices == 0 || adapter.snapshot().paused {
            0
        } else {
            interval
        };
        if advance > 0 {
            context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
            let mut final_context = context.clone();
            final_context.anchor.tick = GameTick(now.checked_add(advance).ok_or_else(|| {
                refusal(
                    ErrorCode::BudgetExceeded,
                    "foreground slice exceeds the game-time horizon",
                )
            })?);
            final_context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
            final_context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        }

        // Only the bounded laboratory is forked. Failed validation, source
        // reads or a later dependent dispatch cannot leave this slice partial.
        let mut next = adapter.clone();
        if advance > 0 {
            next.advance_ticks(advance)?;
        }
        let mut receipts = Vec::with_capacity(tracked_actions.len());
        let mut dispatches = self.dispatches;
        for action_id in tracked_actions {
            self.check_deadline(context)?;
            let was_prepared = next
                .action_receipt(*action_id)
                .is_some_and(|receipt| receipt.state == CommitState::Prepared);
            let poll_context = OperationContext {
                anchor: next.snapshot().anchor(),
                ..context.clone()
            };
            let receipt = next.poll_action(*action_id, &poll_context)?;
            if was_prepared
                && receipt.state != CommitState::Prepared
                && !matches!(
                    next.action_work_state(*action_id)?,
                    dfmcp_intent::EffectWorkState::NeverDispatched
                )
            {
                dispatches = dispatches.checked_add(1).ok_or_else(|| {
                    refusal(
                        ErrorCode::BudgetExceeded,
                        "foreground dispatch counter exhausted",
                    )
                })?;
                if dispatches > self.max_actions.min(context.budget.max_actions) {
                    return Err(refusal(
                        ErrorCode::BudgetExceeded,
                        "foreground wait exceeds its aggregate deferred-action budget",
                    ));
                }
            }
            receipts.push(receipt);
        }
        self.check_deadline(context)?;
        self.advanced_ticks += advance;
        self.slices += 1;
        self.polls = polls;
        self.dispatches = dispatches;
        self.expected_anchor = next.snapshot().anchor();
        if self.tracked.is_none() {
            self.tracked = Some(selected);
        }
        self.finished = self.remaining_game_ticks() == 0 || next.snapshot().paused;
        let result = ProgressSlice {
            receipts,
            finished: self.finished,
        };
        *adapter = next;
        Ok(Some(result))
    }
}

#[cfg(test)]
#[path = "lab_progress_tests.rs"]
mod tests;
