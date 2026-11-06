//! Observe one selected production objective while retaining the entire parent.
//! Historical labor receipts protect sequencing custody; they are never current
//! labor evidence and cannot satisfy a parent's current terminal predicate.

use std::path::Path;

use dfmcp_core::{ObservationCursor, StateAnchor};
use dfmcp_intent::{Action, ObligationStatus};
use dfmcp_world::{EvidencePolicy, PredicateTruth};

use crate::build_placement::journal::private_file::PrivateBuildFile;
use crate::semantic_work_orders::goal_history::open_private_selected_goal_history_store;
use crate::semantic_work_orders::{
    DurableWorkOrderGoalMonitor, GoalHistoryStore, OperationsGoalSource, SemanticWorkOrderReview,
    WorkOrderGoalProgress, WorkOrderGoalProjection, WorkOrderGoalResult,
};
use crate::work_order_progress::ProgressSource;

use super::*;

/// An exclusive selected-step proof owner. The private inner monitor cannot be
/// passed to the legacy one-step APIs or detached from its predecessor custody.
pub struct SelectedWorkOrderGoalMonitor<H> {
    pub(super) step: StepId,
    pub(super) portfolio: bool,
    pub(super) archive: PlanArchiveSeal,
    pub(super) dependencies: Dependencies,
    pub(super) parent_workforce_floor: Option<crate::workforce_control::WorkforceCapture>,
    pub(super) parent_workforce_fenced: bool,
    pub(super) parent_high_tick: GameTick,
    inner: DurableWorkOrderGoalMonitor<H>,
}
impl<H: EffectJournalStorage> SelectedWorkOrderGoalMonitor<H> {
    pub const fn step_id(&self) -> StepId {
        self.step
    }
    pub const fn archive_seal(&self) -> PlanArchiveSeal {
        self.archive
    }
    pub fn original_plan(&self) -> &PreparedPlan {
        self.inner.original_plan()
    }
    pub fn review_seal(&self) -> Digest32 {
        self.inner.review_seal()
    }
    /// The allocation retained by the exact original selected review, used only
    /// for an independently authorized whole-portfolio observation bracket.
    pub fn native_order_id(&self) -> u32 {
        self.inner.native_order_id()
    }
    /// `current_goal` refers to the selected step. `plan_terminal` retains the
    /// separate truth of the complete parent's original terminal predicate.
    pub fn latest(&self) -> Option<&WorkOrderGoalProgress> {
        self.inner.latest()
    }
    pub fn first_satisfied_anchor(&self) -> Option<StateAnchor> {
        self.inner.first_satisfied_anchor()
    }
    pub fn last_observation_anchor(&self) -> Option<StateAnchor> {
        self.inner.last_observation_anchor()
    }
    pub fn high_cursor(&self) -> ObservationCursor {
        self.inner.high_cursor()
    }
    pub fn high_tick(&self) -> GameTick {
        self.inner.high_tick().max(self.parent_high_tick)
    }
    pub fn is_fenced(&self) -> bool {
        self.inner.is_fenced()
    }
    /// Current-parent source lineage is separate from the selected inventory
    /// history. A source replacement/regression cannot be repaired by retrying
    /// another parent sample in this process.
    pub const fn parent_observation_fenced(&self) -> bool {
        self.parent_workforce_fenced
    }
    pub fn is_read_only(&self) -> bool {
        self.inner.history_store().is_read_only()
    }
    pub fn history_event_count(&self) -> usize {
        self.inner.history_event_count()
    }
    pub fn history_byte_len(&self) -> usize {
        self.inner.history_byte_len()
    }
    pub fn historical_obligation_status(&self) -> Option<&ObligationStatus> {
        self.inner.historical_obligation_status()
    }
    /// A failed outer host response also withdraws current proof and unfinished
    /// stability. The next authorized poll persists the interruption.
    pub fn invalidate_current_sample(&mut self) -> Result<()> {
        self.inner.invalidate_current_sample()
    }
}

#[derive(Clone, Debug)]
pub struct SelectedWorkOrderGoalResult {
    pub(super) inner: WorkOrderGoalResult,
}
impl SelectedWorkOrderGoalResult {
    pub fn progress(&self) -> &WorkOrderGoalProgress {
        self.inner.progress()
    }
    pub fn creation_result(&self) -> &NativeOrderCreation {
        self.inner.creation_result()
    }
    pub fn native_state(&self) -> CreationState {
        self.inner.native_state()
    }
    pub fn selected_goal_proven(&self) -> bool {
        self.inner.progress().current_goal_proven()
            && matches!(self.creation_result(), NativeOrderCreation::Inserted { .. })
    }
    pub fn original_goal_proven(&self) -> bool {
        self.selected_goal_proven() && self.inner.progress().plan_terminal == PredicateTruth::True
    }
}

/// Each policy invocation is bracketed by separately prepaid predecessor
/// custody checks. Source callbacks cannot replace a dependency between proof
/// evaluation and durable publication.
struct GuardedPolicy<'a, WJ, WA, A> {
    workforce: &'a mut SemanticWorkforceSession<WJ, WA>,
    dependencies: &'a Dependencies,
    checks: Vec<OperationContext>,
    until: Instant,
    policy: A,
}
impl<WJ: EffectJournalStorage, WA: EffectJournalStorage, A> GuardedPolicy<'_, WJ, WA, A>
where
    A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
{
    fn verify(&mut self, c: &OperationContext) -> Result<()> {
        let mut check = self.checks.pop().ok_or_else(exhausted)?;
        if check.session_id != c.session_id || check.request_id != c.request_id {
            return Err(custody());
        }
        check.grants = c.grants.clone();
        check.cancellation_requested |= c.cancellation_requested;
        check.anchor.tick = check
            .anchor
            .tick
            .max(c.anchor.tick)
            .max(GameTick(self.workforce.high_tick()));
        let remaining = self
            .until
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?;
        let millis = u64::try_from(remaining.as_millis()).map_err(|_| exhausted())?;
        if millis == 0 {
            return Err(exhausted());
        }
        check.budget.max_wall_millis = check
            .budget
            .max_wall_millis
            .min(c.budget.max_wall_millis)
            .min(millis);
        check.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        self.dependencies.verify(self.workforce, &check)
    }
    fn authorize(
        &mut self,
        projection: &WorkOrderGoalProjection,
        c: &OperationContext,
    ) -> Result<EvidencePolicy> {
        self.verify(c)?;
        let policy = (self.policy)(projection, c)?;
        self.verify(c)?;
        Ok(policy)
    }
}

impl<WJ, WA, OJ, OA, ON, E, F> Executor<WJ, WA, OJ, OA, ON, E, F>
where
    WJ: EffectJournalStorage,
    WA: EffectJournalStorage,
    OJ: EffectJournalStorage,
    OA: EffectJournalStorage,
    ON: WorkOrderSource,
    E: WorkforceEvidenceOwner + WorkOrderEvidenceOwner,
    F: WorkforceFactory,
{
    pub(super) fn selected_goal_review(
        &mut self,
        step: StepId,
        call: &mut Call,
    ) -> Result<(SemanticWorkOrderReview, Dependencies)> {
        call.current()?
            .authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        let (result, dependencies) = self.selected_order_custody(step, call)?;
        Ok((result.review().clone(), dependencies))
    }

    fn selected_order_custody(
        &mut self,
        step: StepId,
        call: &mut Call,
    ) -> Result<(SemanticWorkOrderResult, Dependencies)> {
        let selected = policy::selected(&self.original, step)?.clone();
        if policy::route(&selected)?.family != LiveFamily::WorkOrdersV1_10 {
            return Err(error(
                ErrorCode::AdapterRejected,
                "selected goal requires an original finite work-order step",
            ));
        }
        let inventory = self.inventory(call)?;
        let original_record = inventory
            .orders
            .iter()
            .find(|record| record.plan().key() == selected.idempotency_key)
            .ok_or_else(blocked)?;
        if original_record.state() != CreationState::Created {
            return Err(blocked());
        }
        let dependencies = self.dependencies(&selected, &inventory, call)?;
        let review = self
            .orders
            .reattach_selected(self.selection(step)?, &call.take(ORDER_VIEW_BYTES)?)?;
        self.retain_floor(call);
        let inspected = self
            .orders
            .inspect(review.seal(), &call.take(ORDER_VIEW_BYTES)?);
        self.retain_floor(call);
        let inspected = inspected?;
        if inspected.native_record() != original_record
            || !matches!(
                inspected.creation_result(),
                NativeOrderCreation::Inserted { .. }
            )
            || review.original_plan() != &self.original
            || review.step_id() != step
            || review.selected_step() != &selected
        {
            return Err(custody());
        }
        Ok((inspected, dependencies))
    }

    /// Inspect the original verified insertion and all original predecessor
    /// receipts under Query alone. This reads the existing private custodians;
    /// it performs no native observation, preparation, reconciliation or effect.
    /// A recovered review is historical identity, never a commit permit.
    pub fn inspect_selected_order_custody(
        &mut self,
        step: StepId,
        context: &OperationContext,
    ) -> Result<SemanticWorkOrderResult> {
        let mut call = self.call(context)?;
        let result = (|| {
            let (inspected, dependencies) = self.selected_order_custody(step, &mut call)?;
            let publication = call.take(WORKFORCE_VIEW_BYTES + ORDER_VIEW_BYTES + 2 * MIB)?;
            let dependency_check = call.take(dependencies.check_bytes)?;
            self.final_check(&publication, &mut call)?;
            dependencies.verify(
                &mut self.workforce,
                &call.edge(&dependency_check, self.high_tick)?,
            )?;
            self.retain_floor(&mut call);
            call.current()?
                .authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            Ok(inspected)
        })();
        self.retain_floor(&mut call);
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }

    /// Open or recover a selected objective over caller-owned bounded storage.
    /// Only a proved original insertion with every original labor predecessor
    /// can acquire this owner. Recovery performs no native I/O or redispatch.
    pub fn begin_selected_order_goal_monitor<H, A>(
        &mut self,
        step: StepId,
        storage: H,
        initialize: bool,
        read_only: bool,
        context: &OperationContext,
        authorize_history: A,
    ) -> Result<SelectedWorkOrderGoalMonitor<H>>
    where
        H: EffectJournalStorage,
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        self.begin_selected_order_goal_with(
            step,
            context,
            |journal, review, c| {
                GoalHistoryStore::open_selected(storage, journal, review, initialize, read_only, c)
            },
            authorize_history,
            false,
        )
    }

    /// The path is trusted operator configuration, never a plan or MCP argument.
    pub fn open_private_selected_order_goal_monitor<A>(
        &mut self,
        step: StepId,
        path: &Path,
        initialize: bool,
        read_only: bool,
        context: &OperationContext,
        authorize_history: A,
    ) -> Result<SelectedWorkOrderGoalMonitor<PrivateBuildFile>>
    where
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        self.begin_selected_order_goal_with(
            step,
            context,
            |journal, review, c| {
                open_private_selected_goal_history_store(
                    path, journal, review, initialize, read_only, c,
                )
            },
            authorize_history,
            false,
        )
    }

    /// Open one exact original portfolio leaf in its own strict source-cursor
    /// lane. The complete parent owns at most four such lanes; this policy is
    /// never reconstructed from caller-supplied history or a selected result.
    pub fn open_private_portfolio_order_goal_monitor<A>(
        &mut self,
        step: StepId,
        path: &Path,
        initialize: bool,
        read_only: bool,
        context: &OperationContext,
        authorize_history: A,
    ) -> Result<SelectedWorkOrderGoalMonitor<PrivateBuildFile>>
    where
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        let first = self.original.steps.first().ok_or_else(custody)?;
        if !(2..=5).contains(&self.original.steps.len())
            || !matches!(first.action, Action::SetLabor { .. })
            || self.original.steps.iter().skip(1).any(|leaf| {
                !matches!(leaf.action, Action::CreateWorkOrder { .. })
                    || leaf.depends_on != [first.id]
            })
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                "portfolio histories require one labor and at most four original order leaves",
            ));
        }
        self.begin_selected_order_goal_with(
            step,
            context,
            |journal, review, c| {
                open_private_selected_goal_history_store(
                    path, journal, review, initialize, read_only, c,
                )
            },
            authorize_history,
            true,
        )
    }

    fn begin_selected_order_goal_with<H, O, A>(
        &mut self,
        step: StepId,
        context: &OperationContext,
        open_history: O,
        authorize_history: A,
        portfolio: bool,
    ) -> Result<SelectedWorkOrderGoalMonitor<H>>
    where
        H: EffectJournalStorage,
        O: FnOnce(
            Digest32,
            &SemanticWorkOrderReview,
            &OperationContext,
        ) -> Result<GoalHistoryStore<H>>,
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        let mut call = self.call(context)?;
        let result = (|| {
            let (review, dependencies) = self.selected_goal_review(step, &mut call)?;
            let publication = call.take(WORKFORCE_VIEW_BYTES + ORDER_VIEW_BYTES + 2 * MIB)?;
            let dependency_check = call.take(dependencies.check_bytes)?;
            let remaining = call.take(call.bytes.checked_sub(1).ok_or_else(exhausted)?)?;
            let inner = self.orders.begin_selected_goal_history(
                review.seal(),
                &remaining,
                open_history,
                authorize_history,
                portfolio,
            );
            self.retain_floor(&mut call);
            let inner = inner?;
            call.floor(inner.high_tick().get());
            self.final_check(&publication, &mut call)?;
            dependencies.verify(
                &mut self.workforce,
                &call.edge(&dependency_check, self.high_tick)?,
            )?;
            self.retain_floor(&mut call);
            call.current()?
                .authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
            Ok(SelectedWorkOrderGoalMonitor {
                step,
                portfolio,
                archive: self.archive,
                dependencies,
                parent_workforce_floor: None,
                parent_workforce_fenced: false,
                parent_high_tick: self.original.anchor.tick,
                inner,
            })
        })();
        self.retain_floor(&mut call);
        result
    }

    /// One fresh sample of the exact selected objective. Source acquisition and
    /// history publication share one diminishing allowance and deadline; no
    /// native prepare, commit, cancellation or clock advance is reachable.
    pub fn poll_selected_order_goal<H, P, O, PF, OF, A>(
        &mut self,
        monitor: &mut SelectedWorkOrderGoalMonitor<H>,
        cursor: ObservationCursor,
        context: &OperationContext,
        progress_factory: PF,
        operations_factory: OF,
        policy: A,
    ) -> Result<SelectedWorkOrderGoalResult>
    where
        H: EffectJournalStorage,
        P: ProgressSource,
        O: OperationsGoalSource,
        PF: FnOnce(&OperationContext) -> Result<P>,
        OF: FnOnce(&OperationContext) -> Result<O>,
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        let result = (|| {
            let mut call = self.call(context)?;
            call.floor(monitor.high_tick().get());
            let result = (|| {
                if monitor.original_plan() != &self.original || monitor.archive != self.archive {
                    return Err(custody());
                }
                let (review, dependencies) = self.selected_goal_review(monitor.step, &mut call)?;
                if review.seal() != monitor.review_seal()
                    || dependencies.journal != monitor.dependencies.journal
                    || dependencies.records != monitor.dependencies.records
                {
                    return Err(custody());
                }
                let publication = call.take(WORKFORCE_VIEW_BYTES + ORDER_VIEW_BYTES + 2 * MIB)?;
                let dependency_check = call.take(dependencies.check_bytes)?;
                let checks = (0..4)
                    .map(|_| call.take(dependencies.check_bytes))
                    .collect::<Result<Vec<_>>>()?;
                let remaining = call.take(call.bytes.checked_sub(1).ok_or_else(exhausted)?)?;
                let mut guarded = GuardedPolicy {
                    workforce: &mut self.workforce,
                    dependencies: &dependencies,
                    checks,
                    until: call.until,
                    policy,
                };
                let sampled = self.orders.poll_selected_goal_durable(
                    &mut monitor.inner,
                    cursor,
                    &remaining,
                    (progress_factory, operations_factory),
                    |p, c| guarded.authorize(p, c),
                );
                self.retain_floor(&mut call);
                call.floor(monitor.high_tick().get());
                let sampled = sampled?;
                self.final_check(&publication, &mut call)?;
                dependencies.verify(
                    &mut self.workforce,
                    &call.edge(&dependency_check, self.high_tick)?,
                )?;
                self.retain_floor(&mut call);
                call.current()?
                    .authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
                Ok(SelectedWorkOrderGoalResult { inner: sampled })
            })();
            self.retain_floor(&mut call);
            self.high_tick = self.high_tick.max(monitor.high_tick().get());
            result
        })();
        if result.is_err() {
            monitor.invalidate_current_sample()?;
        }
        result
    }
}
