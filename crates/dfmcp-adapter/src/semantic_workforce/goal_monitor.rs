//! Foreground original-goal observation through the existing workforce source.
//!
//! This owner never prepares, commits, cancels, or reconciles a native action.
//! Its monitor is process-local. Reopening requires exact semantic reattachment
//! and a new monitor; no stability count, achievement, or authority is restored.

use dfmcp_core::{
    ActionId, Capability, Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext,
    Result, RiskTier, StateAnchor,
};
use dfmcp_intent::{ObligationRuntime, ObligationStatus};
use dfmcp_world::{EvidencePolicy, Predicate, PredicateEvidence, PredicateTruth};

use crate::control_effect_journal::EffectJournalStorage;
use crate::live_routing::LiveRoutingEvidence;
use crate::workforce_control::journal::{
    AssignmentRecord, AssignmentState, MAX_FRAME, WorkforceBinding, WorkforceView,
};
use crate::workforce_control::rpc::{CONNECT_BYTES, RPC_BYTES, WorkforceSource};
use crate::workforce_control::{AssignmentPhase, WorkforceCapture};
use crate::workforce_session::view_cost;

use super::projection::{IdentityBinding, WorkforceGoalProjection};
use super::{
    Call, SemanticWorkforceReview, SemanticWorkforceSession, SingleLaborResult, action_result,
    custody, error, exhausted, original,
};

// A capture has <=32 citizens and <=128 labor columns, hence <=4,192 facts.
// 64 MiB covers construction, bounded canonical encodings, independent policy
// revalidation and retained monitor copies. Neither callback receives this local
// reserve. Source/connection/journal work has its own read-only reservation.
pub(super) const PROJECTION_RESERVE: u64 = 64 * 1024 * 1024;
pub(super) const POLICY_RESERVE: u64 = 1024 * 1024;
const OUTPUT_RESERVE: u64 = 8192;

fn read_reserve(call: &mut Call, view: &WorkforceView) -> Result<OperationContext> {
    // The existing native session owns connection and one ObserveWorkforce RPC.
    // Keep its conservative journal-work allowance, but do not require unused
    // mutation RPCs or two routing refresh reservations for an observation.
    let bytes = 12u64
        .checked_mul(view_cost(view) + 4 * MAX_FRAME as u64)
        .and_then(|bytes| bytes.checked_add(CONNECT_BYTES + RPC_BYTES))
        .ok_or_else(exhausted)?;
    let final_views = 8u64
        .checked_mul(view_cost(view) + MAX_FRAME as u64)
        .ok_or_else(exhausted)?;
    if call.remaining <= bytes.checked_add(final_views).ok_or_else(exhausted)? {
        return Err(exhausted());
    }
    call.reserve(bytes)
}

fn native_sequence_floor(record: &AssignmentRecord) -> Result<u64> {
    let sequence = record.plan().before().sequence();
    if record
        .effect()
        .is_some_and(|effect| effect.phase() == AssignmentPhase::Applied)
    {
        // Applied verifies a full after-capture whose dispatch fence advanced
        // once. It is a clock floor only, never the next observation itself.
        sequence.checked_add(1).ok_or_else(exhausted)
    } else {
        Ok(sequence)
    }
}

pub(super) fn authorize_observation(
    binding: &IdentityBinding,
    original_anchor: StateAnchor,
    context: &OperationContext,
) -> Result<()> {
    if context.anchor.fortress_id != original_anchor.fortress_id
        || context.anchor.cursor.epoch != original_anchor.cursor.epoch
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "original workforce goal cannot cross a fortress or restore epoch",
        ));
    }
    let units = binding.units();
    if units.len() > context.budget.max_entities as usize {
        return Err(exhausted());
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &units, None)
}

#[derive(Clone, Debug)]
pub struct WorkforceGoalProgress {
    pub anchor: StateAnchor,
    pub review_seal: Digest32,
    pub plan_digest: Digest32,
    pub source_digest: Digest32,
    pub postconditions: PredicateTruth,
    pub plan_terminal: PredicateTruth,
    pub obligation_terminal: Option<PredicateTruth>,
    pub failure_predicate: Option<PredicateTruth>,
    /// Conjunction of all original postconditions, plan terminal, and the
    /// optional original obligation terminal, evaluated at this fresh anchor.
    pub current_goal: PredicateTruth,
    /// Historical runtime state. Fulfilled does not make current_goal True.
    pub obligation: Option<ObligationStatus>,
    pub first_satisfied_anchor: Option<StateAnchor>,
    /// False on a missing canonical sequence or an interrupted preceding read.
    pub continuous: bool,
}

impl WorkforceGoalProgress {
    /// Current eligible facts and any required original stability window agree.
    /// This says nothing about an unresolved native action.
    pub fn current_goal_proven(&self) -> bool {
        self.current_goal == PredicateTruth::True
            && self.failure_predicate != Some(PredicateTruth::True)
            && self
                .obligation
                .as_ref()
                .is_none_or(|status| matches!(status, ObligationStatus::Fulfilled { .. }))
    }
}

/// Native action history and fresh goal evidence stay separate. A True goal
/// never turns an Unknown receipt into an Applied or verified action.
#[derive(Clone, Debug)]
pub struct WorkforceGoalResult {
    progress: WorkforceGoalProgress,
    action: SingleLaborResult,
    native_state: AssignmentState,
    native_phase: Option<AssignmentPhase>,
    native_receipt: Option<Digest32>,
}
impl WorkforceGoalResult {
    pub fn progress(&self) -> &WorkforceGoalProgress {
        &self.progress
    }
    pub fn action_result(&self) -> &SingleLaborResult {
        &self.action
    }
    pub fn native_state(&self) -> AssignmentState {
        self.native_state
    }
    pub fn native_phase(&self) -> Option<AssignmentPhase> {
        self.native_phase
    }
    pub fn native_receipt(&self) -> Option<Digest32> {
        self.native_receipt
    }
    pub fn original_goal_proven(&self) -> bool {
        self.progress.current_goal_proven()
    }
    pub fn semantic_completion_proven(&self) -> bool {
        self.original_goal_proven() && matches!(self.action, SingleLaborResult::Verified { .. })
    }
}

/// One original plan, identity mapping and fixed obligation. Construct only
/// through SemanticWorkforceSession::begin_goal_monitor after exact native and
/// semantic custody verification. The observing shell owns canonical cursor
/// allocation and independently issues source policy for every fresh capture.
#[derive(Debug)]
pub struct WorkforceGoalMonitor {
    pub(super) review: SemanticWorkforceReview,
    pub(super) identities: IdentityBinding,
    pub(super) postconditions: Predicate,
    pub(super) conjunction: Predicate,
    pub(super) runtime: Option<ObligationRuntime>,
    pub(super) action_id: ActionId,
    pub(super) last_anchor: Option<StateAnchor>,
    pub(super) cursor_floor: ObservationCursor,
    pub(super) high_tick: GameTick,
    pub(super) high_native_sequence: u64,
    pub(super) first_satisfied_anchor: Option<StateAnchor>,
    pub(super) latest: Option<WorkforceGoalProgress>,
    pub(super) interrupted: bool,
    pub(super) fenced: bool,
}

impl WorkforceGoalMonitor {
    pub(super) fn new(
        review: SemanticWorkforceReview,
        evidence: &LiveRoutingEvidence<'_>,
    ) -> Result<Self> {
        original(&review.original)?;
        let identities = IdentityBinding::new(&review, evidence)?;
        let step = &review.original.steps[0];
        let postconditions = Predicate::All(step.postconditions.clone()).normalized();
        let mut terms = vec![
            postconditions.clone(),
            review.original.terminal_condition.clone(),
        ];
        if let Some(spec) = &step.obligation {
            terms.push(spec.terminal.clone());
        }
        let conjunction = Predicate::All(terms).normalized();
        conjunction.validate_shape()?;
        let number = review.seal.first_u128();
        let action_id = ActionId::new(if number == 0 { 1 } else { number });
        let runtime = if let Some(original_spec) = &step.obligation {
            let mut spec = original_spec.clone();
            // Every credited sample must satisfy the complete original goal,
            // not just the action-specific temporal predicate.
            spec.terminal = conjunction.clone();
            let mut runtime = ObligationRuntime::new();
            runtime.register_obligation(action_id, spec, review.original.anchor.tick)?;
            Some(runtime)
        } else {
            None
        };
        Ok(Self {
            high_tick: review.original.anchor.tick,
            cursor_floor: review.original.anchor.cursor,
            high_native_sequence: review.native.before().sequence(),
            review,
            identities,
            postconditions,
            conjunction,
            runtime,
            action_id,
            last_anchor: None,
            first_satisfied_anchor: None,
            latest: None,
            interrupted: false,
            fenced: false,
        })
    }

    // Only the owner can copy an unfinished temporal streak for atomic
    // publication. Public Clone would let callers branch around a contradiction.
    pub(super) fn pending_sample(&self) -> Self {
        Self {
            review: self.review.clone(),
            identities: self.identities.clone(),
            postconditions: self.postconditions.clone(),
            conjunction: self.conjunction.clone(),
            runtime: self.runtime.clone(),
            action_id: self.action_id,
            last_anchor: self.last_anchor,
            cursor_floor: self.cursor_floor,
            high_tick: self.high_tick,
            high_native_sequence: self.high_native_sequence,
            first_satisfied_anchor: self.first_satisfied_anchor,
            latest: self.latest.clone(),
            interrupted: self.interrupted,
            fenced: self.fenced,
        }
    }

    pub fn review_seal(&self) -> Digest32 {
        self.review.seal
    }
    pub fn original_plan(&self) -> &dfmcp_intent::PreparedPlan {
        &self.review.original
    }
    /// Last successfully published sample in this process, never an implicit
    /// fresh read. Every failed poll clears it before returning.
    pub fn latest(&self) -> Option<&WorkforceGoalProgress> {
        self.latest.as_ref()
    }
    pub fn first_satisfied_anchor(&self) -> Option<StateAnchor> {
        self.first_satisfied_anchor
    }
    pub fn last_observation_anchor(&self) -> Option<StateAnchor> {
        self.last_anchor
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }

    /// Signal an independently interrupted read. Preserve absolute deadline,
    /// cadence floor and immutable historical fulfillment; erase only a pending
    /// stability streak and the previous sample's claim to be current.
    pub fn observation_interrupted(&mut self) -> Result<()> {
        self.latest = None;
        self.interrupted = true;
        if let Some(runtime) = &mut self.runtime {
            runtime.observation_interrupted(self.action_id)?;
        }
        Ok(())
    }

    fn cursor(&self, cursor: ObservationCursor) -> Result<()> {
        let original = self.review.original.anchor.cursor;
        let previous = self
            .last_anchor
            .map_or(self.cursor_floor, |anchor| anchor.cursor);
        if self.fenced
            || cursor.epoch != original.epoch
            || cursor.sequence < previous.sequence
            || cursor.sequence < self.cursor_floor.sequence
            || (self.last_anchor.is_none() && cursor.sequence <= self.cursor_floor.sequence)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "workforce goal cursor regressed, crossed a restore, or belongs to a fenced monitor",
            ));
        }
        Ok(())
    }

    pub(super) fn sample(
        &mut self,
        projection: &WorkforceGoalProjection,
        evidence: &PredicateEvidence<'_>,
    ) -> Result<WorkforceGoalProgress> {
        let anchor = projection.anchor();
        self.cursor(anchor.cursor)?;
        if anchor.tick < self.high_tick
            || self.last_anchor.is_some_and(|last| {
                anchor.tick < last.tick || (anchor.cursor == last.cursor && anchor != last)
            })
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "workforce goal observation regressed or forked an accepted canonical anchor",
            ));
        }
        let basis = self.last_anchor.unwrap_or(self.review.original.anchor);
        let consecutive = anchor.cursor == basis.cursor
            || basis.cursor.sequence.checked_add(1) == Some(anchor.cursor.sequence);
        let continuous = consecutive && !self.interrupted;
        if !continuous && let Some(runtime) = &mut self.runtime {
            runtime.observation_interrupted(self.action_id)?;
        }
        let step = &self.review.original.steps[0];
        let postconditions = evidence.evaluate(&self.postconditions)?;
        let plan_terminal = evidence.evaluate(&self.review.original.terminal_condition)?;
        let obligation_terminal = step
            .obligation
            .as_ref()
            .map(|spec| evidence.evaluate(&spec.terminal))
            .transpose()?;
        let failure_predicate = step
            .obligation
            .as_ref()
            .and_then(|spec| spec.failure.as_ref())
            .map(|predicate| evidence.evaluate(predicate))
            .transpose()?;
        let current_goal = evidence.evaluate(&self.conjunction)?;
        if let Some(runtime) = &mut self.runtime {
            runtime.step_tick_with_evidence(evidence)?;
        }
        let obligation = self
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.get_status(self.action_id))
            .cloned();
        let mut progress = WorkforceGoalProgress {
            anchor,
            review_seal: self.review.seal,
            plan_digest: self.review.original.digest,
            source_digest: projection.source_digest(),
            postconditions,
            plan_terminal,
            obligation_terminal,
            failure_predicate,
            current_goal,
            obligation,
            first_satisfied_anchor: self.first_satisfied_anchor,
            continuous,
        };
        if progress.current_goal_proven() && self.first_satisfied_anchor.is_none() {
            self.first_satisfied_anchor = Some(anchor);
            progress.first_satisfied_anchor = Some(anchor);
        }
        self.last_anchor = Some(anchor);
        self.high_tick = self.high_tick.max(anchor.tick);
        self.high_native_sequence = projection.capture().sequence();
        self.interrupted = false;
        self.latest = Some(progress.clone());
        Ok(progress)
    }
}

/// Internal publication hooks. Only the durable owner implements persistence;
/// callers cannot inject a publication sink or import a proof monitor.
pub(super) trait GoalPublication {
    fn started(
        &mut self,
        monitor: &WorkforceGoalMonitor,
        cursor: ObservationCursor,
        call: &mut Call,
    ) -> Result<()>;
    fn captured(&mut self, capture: &WorkforceCapture, call: &mut Call) -> Result<()>;
    fn published(
        &mut self,
        projection: &WorkforceGoalProjection,
        policy: &EvidencePolicy,
        call: &mut Call,
    ) -> Result<()>;
    fn failed(&mut self, monitor: &WorkforceGoalMonitor, context: &OperationContext) -> Result<()>;
}
struct ProcessLocalPublication;
impl GoalPublication for ProcessLocalPublication {
    fn started(
        &mut self,
        _: &WorkforceGoalMonitor,
        _: ObservationCursor,
        _: &mut Call,
    ) -> Result<()> {
        Ok(())
    }
    fn captured(&mut self, _: &WorkforceCapture, _: &mut Call) -> Result<()> {
        Ok(())
    }
    fn published(
        &mut self,
        _: &WorkforceGoalProjection,
        _: &EvidencePolicy,
        _: &mut Call,
    ) -> Result<()> {
        Ok(())
    }
    fn failed(&mut self, _: &WorkforceGoalMonitor, _: &OperationContext) -> Result<()> {
        Ok(())
    }
}

impl<S: EffectJournalStorage, B: EffectJournalStorage> SemanticWorkforceSession<S, B> {
    /// Start a process-local proof monitor without creating or changing native
    /// work. The exact original typed identity evidence is required, including
    /// after restart; archived receipt labor masks are never observation inputs.
    pub fn begin_goal_monitor(
        &mut self,
        seal: Digest32,
        original_identity: &LiveRoutingEvidence<'_>,
        context: &OperationContext,
    ) -> Result<WorkforceGoalMonitor> {
        let mut call = Call::new(context)?;
        call.reserve(PROJECTION_RESERVE)?;
        let view = self.view(&mut call)?;
        let review = self.attached(seal)?;
        self.exact_association(&review)?;
        let record = view
            .records
            .iter()
            .find(|record| record.plan() == &review.native)
            .ok_or_else(custody)?;
        let sequence_floor = native_sequence_floor(record)?;
        let mut monitor = WorkforceGoalMonitor::new(review, original_identity)?;
        monitor.high_native_sequence = sequence_floor;
        monitor.high_tick = monitor.high_tick.max(call.context()?.anchor.tick);
        authorize_observation(
            &monitor.identities,
            monitor.review.original.anchor,
            &call.context()?,
        )?;
        // A restarted monitor forgets proof history, never the observing shell's
        // already allocated canonical sequence.
        if context.anchor.cursor.sequence > monitor.cursor_floor.sequence {
            monitor.cursor_floor = context.anchor.cursor;
        }
        self.view(&mut call)?;
        authorize_observation(
            &monitor.identities,
            monitor.review.original.anchor,
            &call.context()?,
        )?;
        Ok(monitor)
    }

    /// Acquire one fresh native selected-citizen capture, close its source,
    /// obtain independently issued projection policy, and publish one bounded
    /// original-goal sample. Every call is explicit; there is no polling worker,
    /// clock advancement, action dispatch, retry, or authority restoration.
    ///
    /// The policy callback runs twice under separately reserved shrinking
    /// contexts, after the source has closed. Its policy must remain identical
    /// through final custody validation. It may grant a strict subset of facts;
    /// missing grants produce Unknown rather than an invented observation.
    pub fn poll_original_goal<N, F, A>(
        &mut self,
        monitor: &mut WorkforceGoalMonitor,
        cursor: ObservationCursor,
        context: &OperationContext,
        factory: F,
        mut authorize: A,
    ) -> Result<WorkforceGoalResult>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
        A: FnMut(&WorkforceGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        self.poll_original_goal_with_publication(
            monitor,
            cursor,
            context,
            factory,
            &mut authorize,
            &mut ProcessLocalPublication,
        )
    }

    pub(super) fn poll_original_goal_with_publication<N, F, A, P>(
        &mut self,
        monitor: &mut WorkforceGoalMonitor,
        cursor: ObservationCursor,
        context: &OperationContext,
        factory: F,
        mut authorize: A,
        publication: &mut P,
    ) -> Result<WorkforceGoalResult>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
        A: FnMut(&WorkforceGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
        P: GoalPublication,
    {
        monitor.latest = None;
        let result = (|| {
            let mut call = Call::new(context)?;
            call.context.anchor.tick = call.context.anchor.tick.max(monitor.high_tick);
            authorize_observation(
                &monitor.identities,
                monitor.review.original.anchor,
                &call.context()?,
            )?;
            if context.anchor.cursor.sequence > monitor.cursor_floor.sequence {
                monitor.cursor_floor = context.anchor.cursor;
            }
            monitor.cursor(cursor)?;
            if OUTPUT_RESERVE > u64::from(context.budget.max_output_tokens) * 4 {
                return Err(exhausted());
            }
            call.reserve(OUTPUT_RESERVE)?;
            call.reserve(PROJECTION_RESERVE)?;
            let policy_budget = call.reserve(POLICY_RESERVE)?;
            let final_policy_budget = call.reserve(POLICY_RESERVE)?;
            let view = self.view(&mut call)?;
            let review = self.attached(monitor.review.seal)?;
            self.exact_association(&review)?;
            if review.original.digest != monitor.review.original.digest
                || review.native != monitor.review.native
                || !view
                    .records
                    .iter()
                    .any(|record| record.plan() == &review.native)
            {
                return Err(custody());
            }
            let record = view
                .records
                .iter()
                .find(|record| record.plan() == &review.native)
                .ok_or_else(custody)?;
            monitor.high_native_sequence = monitor
                .high_native_sequence
                .max(native_sequence_floor(record)?);
            let ids = monitor.identities.native_ids();
            let native_context = read_reserve(&mut call, &view)?;
            publication.started(monitor, cursor, &mut call)?;
            let store = &mut self.associations;
            let identities = &monitor.identities;
            let original_anchor = review.original.anchor;
            let guard_call = &mut call;
            let guarded_factory = |binding: &WorkforceBinding, c: &OperationContext| {
                store.verify(&guard_call.context()?)?;
                let current = guard_call.edge(c)?;
                authorize_observation(identities, original_anchor, &current)?;
                let source = factory(binding, &current)?;
                store.verify(&guard_call.context()?)?;
                authorize_observation(identities, original_anchor, &guard_call.context()?)?;
                Ok(source)
            };
            let acquisition = self.native.observe(&ids, &native_context, guarded_factory);
            // A read-only monitor never leaves a native planning selection.
            self.native.clear_selection();
            monitor.high_tick = monitor.high_tick.max(GameTick(self.native.high_tick()));
            let capture = acquisition?;
            // Save raw evidence and its floors before source policy can refuse
            // publication. Persistence confers no observation authority.
            publication.captured(&capture, &mut call)?;
            call.context.anchor.tick = call.context.anchor.tick.max(GameTick(capture.tick()));
            monitor.high_tick = monitor.high_tick.max(GameTick(self.native.high_tick()));
            authorize_observation(&monitor.identities, original_anchor, &call.context()?)?;
            if capture.tick() < call.context.anchor.tick.get()
                || capture.sequence() < monitor.high_native_sequence
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "workforce goal source regressed below its observation or authority floor",
                ));
            }
            if let Err(cause) = monitor.identities.validate_capture(&capture) {
                monitor.fenced = true;
                return Err(cause);
            }
            // Preserve the observed source clock even if later publication is
            // denied. Failed policy cannot roll back the authority floor.
            monitor.high_native_sequence = capture.sequence();
            let projection = monitor.identities.project(capture, cursor)?;
            if cursor == context.anchor.cursor && projection.anchor() != context.anchor {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "workforce goal observation forked the caller's current canonical anchor",
                ));
            }
            let mut policy_context = call.edge(&policy_budget)?;
            policy_context.anchor = projection.anchor();
            authorize_observation(&monitor.identities, original_anchor, &policy_context)?;
            let policy = authorize(&projection, &policy_context)?;
            let evidence = projection.evidence(policy.clone())?;
            let mut pending = monitor.pending_sample();
            let progress = pending.sample(&projection, &evidence)?;
            let final_view = self.view(&mut call)?;
            self.exact_association(&review)?;
            let record = final_view
                .records
                .iter()
                .find(|record| record.plan() == &review.native)
                .ok_or_else(custody)?;
            let output = WorkforceGoalResult {
                progress,
                action: action_result(&review, record)?,
                native_state: record.state(),
                native_phase: record.effect().map(|effect| effect.phase()),
                native_receipt: record.effect().map(|effect| effect.receipt()),
            };
            let mut final_context = call.edge(&final_policy_budget)?;
            final_context.anchor = projection.anchor();
            authorize_observation(&monitor.identities, original_anchor, &final_context)?;
            if authorize(&projection, &final_context)? != policy {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "workforce goal source policy changed before publication",
                ));
            }
            self.view(&mut call)?;
            self.exact_association(&review)?;
            authorize_observation(&monitor.identities, original_anchor, &call.context()?)?;
            publication.published(&projection, &policy, &mut call)?;
            *monitor = pending;
            Ok(output)
        })();
        if result.is_err() {
            self.native.clear_selection();
            monitor.observation_interrupted()?;
            if let Err(cause) = publication.failed(monitor, context) {
                monitor.fenced = true;
                return Err(cause);
            }
        }
        result
    }
}
