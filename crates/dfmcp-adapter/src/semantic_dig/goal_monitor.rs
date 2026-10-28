//! Bounded, foreground proof of the exact original excavation goal.
//!
//! Historical designation, current excavation, original predicate truth and
//! temporal achievement remain separate. Monitoring never dispatches an effect.

use super::goal_projection::DigGoalProjection;
use super::{
    Call, LOCAL_BYTES, NativeDigDesignation, SemanticDigReview, SemanticDigSession, custody, error,
    exhausted, original,
};
use crate::control_effect_journal::EffectJournalStorage;
use crate::dig_designation::journal::{DigBinding, DigMode, DigRecord, DigState};
use crate::dig_designation::rpc::DigSource;
use crate::dig_designation::{DigEffect, DigPhase, DigPlan};
use crate::live_spatial::citizens::LiveSpatialCitizenObservation;
use dfmcp_core::{
    ActionId, Capability, Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext,
    Result, RiskTier, StateAnchor,
};
use dfmcp_intent::{ObligationRuntime, ObligationStatus};
use dfmcp_world::{EvidencePolicy, Predicate, PredicateEvidence, PredicateTruth};
use std::sync::{Arc, Weak};

pub(super) const PROJECTION_BYTES: u64 = 64 * 1024 * 1024;
pub(super) const POLICY_BYTES: u64 = 1024 * 1024;
pub(super) const SOURCE_BYTES: u64 = 384 * 1024 * 1024;
const CONNECT_BYTES: u64 = 1024 * 1024;
const OUTPUT_BYTES: u64 = 8192;

/// A trusted host owns acquisition, credentials and the one-connection bracket.
/// Serialized samples alone do not establish native observation authority.
pub trait DigGoalSource {
    fn read_excavation(
        &mut self,
        original: &DigPlan,
        context: &OperationContext,
    ) -> Result<LiveSpatialCitizenObservation>;
}

pub(super) fn authorize(anchor: StateAnchor, context: &OperationContext) -> Result<()> {
    if context.anchor.fortress_id != anchor.fortress_id
        || context.anchor.cursor.epoch != anchor.cursor.epoch
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "original dig goal cannot cross a fortress or restore epoch",
        ));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
}

pub(super) fn edge(call: &Call, reserved: &OperationContext) -> Result<OperationContext> {
    let mut context = call.current()?;
    context.budget.max_bytes = reserved.budget.max_bytes;
    Ok(context)
}

#[derive(Clone, Debug)]
pub struct DigGoalProgress {
    pub anchor: StateAnchor,
    pub review_seal: Digest32,
    pub plan_digest: Digest32,
    pub source_digest: Digest32,
    pub postconditions: PredicateTruth,
    pub plan_terminal: PredicateTruth,
    pub obligation_terminal: Option<PredicateTruth>,
    pub failure_predicate: Option<PredicateTruth>,
    pub current_goal: PredicateTruth,
    pub obligation: Option<ObligationStatus>,
    pub first_satisfied_anchor: Option<StateAnchor>,
    pub receipt_linked: bool,
    pub excavation_condition: &'static str,
    pub excavation_proven: bool,
    pub continuous: bool,
}
impl DigGoalProgress {
    pub fn current_goal_proven(&self) -> bool {
        self.current_goal == PredicateTruth::True
            && self.failure_predicate != Some(PredicateTruth::True)
            && self
                .obligation
                .as_ref()
                .is_none_or(|state| matches!(state, ObligationStatus::Fulfilled { .. }))
    }
}

#[derive(Clone, Debug)]
pub struct DigGoalResult {
    progress: DigGoalProgress,
    designation: NativeDigDesignation,
}
impl DigGoalResult {
    pub fn progress(&self) -> &DigGoalProgress {
        &self.progress
    }
    pub fn designation_result(&self) -> &NativeDigDesignation {
        &self.designation
    }
    pub fn original_goal_proven(&self) -> bool {
        self.progress.current_goal_proven()
    }
    pub fn semantic_completion_proven(&self) -> bool {
        self.original_goal_proven()
            && self.progress.excavation_proven
            && matches!(self.designation, NativeDigDesignation::Designated { .. })
    }
}

/// Consumed source identity/cursor/tick survives policy denial and owner replacement.
#[derive(Clone, Debug, Default)]
pub(super) struct DigSourceFloor {
    pub(super) source: Option<Digest32>,
    pub(super) cursor: Option<ObservationCursor>,
    pub(super) high_tick: GameTick,
    pub(super) fenced: bool,
}
impl DigSourceFloor {
    pub(super) fn merge_retained(&mut self, other: &Self) -> Result<()> {
        self.fenced |= other.fenced;
        self.high_tick = self.high_tick.max(other.high_tick);
        if self.source.zip(other.source).is_some_and(|(a, b)| a != b)
            || self
                .cursor
                .zip(other.cursor)
                .is_some_and(|(a, b)| a.epoch != b.epoch)
        {
            self.fenced = true;
            return Err(custody());
        }
        if self.source.is_none() {
            self.source = other.source;
        }
        if let Some(cursor) = other.cursor {
            if self.cursor.is_none_or(|old| old.sequence < cursor.sequence) {
                self.cursor = Some(cursor);
            }
        }
        Ok(())
    }
    pub(super) fn consume(&mut self, cursor: ObservationCursor) -> Result<()> {
        if self.fenced
            || self
                .cursor
                .is_some_and(|old| old.epoch != cursor.epoch || old.sequence >= cursor.sequence)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original excavation source cursor regressed",
            ));
        }
        self.cursor = Some(cursor);
        Ok(())
    }
    pub(super) fn retain(
        &mut self,
        review: &SemanticDigReview,
        projection: &DigGoalProjection,
        record: &DigEffect,
    ) -> Result<()> {
        let identity = projection.source_identity_digest();
        if projection.review_seal() != review.seal()
            || record.phase() != DigPhase::Designated
            || identity != review.source_identity_digest()
            || self.source.is_some_and(|old| old != identity)
            || projection.anchor().tick < self.high_tick
            || self.cursor != Some(projection.anchor().cursor)
        {
            self.fenced = true;
            return Err(error(
                ErrorCode::StaleAnchor,
                "original excavation source changed or regressed",
            ));
        }
        self.source = Some(identity);
        self.high_tick = self.high_tick.max(projection.anchor().tick);
        Ok(())
    }
}

/// Exclusive proof owner. Reattachment preserves the original deadline and
/// resets unfinished stability; no public clone can branch around a denial.
#[derive(Debug)]
pub struct DigGoalMonitor {
    pub(super) owner: Weak<()>,
    pub(super) review: SemanticDigReview,
    postconditions: Predicate,
    conjunction: Predicate,
    pub(super) runtime: Option<ObligationRuntime>,
    pub(super) action_id: ActionId,
    pub(super) cursor_floor: ObservationCursor,
    pub(super) high_tick: GameTick,
    pub(super) last_anchor: Option<StateAnchor>,
    pub(super) first_satisfied_anchor: Option<StateAnchor>,
    latest: Option<DigGoalProgress>,
    pub(super) interrupted: bool,
    pub(super) fenced: bool,
}
impl DigGoalMonitor {
    pub(super) fn new(
        review: SemanticDigReview,
        context: &OperationContext,
        owner: Weak<()>,
    ) -> Result<Self> {
        let step = &review.original_plan().steps[0];
        let postconditions = Predicate::All(step.postconditions.clone()).normalized();
        let mut terms = vec![
            postconditions.clone(),
            review.original_plan().terminal_condition.clone(),
        ];
        if let Some(spec) = &step.obligation {
            terms.push(spec.terminal.clone());
        }
        let conjunction = Predicate::All(terms).normalized();
        conjunction.validate_shape()?;
        let action_id = ActionId::new(review.seal().first_u128().max(1));
        let runtime = if let Some(spec) = &step.obligation {
            let mut spec = spec.clone();
            spec.terminal = conjunction.clone();
            let mut runtime = ObligationRuntime::new();
            runtime.register_obligation(action_id, spec, review.original_plan().anchor.tick)?;
            Some(runtime)
        } else {
            None
        };
        Ok(Self {
            owner,
            cursor_floor: ObservationCursor {
                epoch: review.original_plan().anchor.cursor.epoch,
                sequence: review
                    .original_plan()
                    .anchor
                    .cursor
                    .sequence
                    .max(context.anchor.cursor.sequence),
            },
            high_tick: review.original_plan().anchor.tick.max(context.anchor.tick),
            review,
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
    pub(super) fn pending_sample(&self) -> Self {
        Self {
            owner: self.owner.clone(),
            review: self.review.clone(),
            postconditions: self.postconditions.clone(),
            conjunction: self.conjunction.clone(),
            runtime: self.runtime.clone(),
            action_id: self.action_id,
            cursor_floor: self.cursor_floor,
            high_tick: self.high_tick,
            last_anchor: self.last_anchor,
            first_satisfied_anchor: self.first_satisfied_anchor,
            latest: self.latest.clone(),
            interrupted: self.interrupted,
            fenced: self.fenced,
        }
    }
    pub fn original_plan(&self) -> &dfmcp_intent::PreparedPlan {
        self.review.original_plan()
    }
    pub fn review_seal(&self) -> Digest32 {
        self.review.seal()
    }
    pub fn latest(&self) -> Option<&DigGoalProgress> {
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
    pub fn observation_interrupted(&mut self) -> Result<()> {
        self.latest = None;
        self.interrupted = true;
        if let Some(runtime) = &mut self.runtime {
            runtime.observation_interrupted(self.action_id)?;
        }
        Ok(())
    }
    pub(super) fn cursor(&self, cursor: ObservationCursor) -> Result<()> {
        if self.fenced
            || cursor.epoch != self.review.original_plan().anchor.cursor.epoch
            || cursor.sequence <= self.cursor_floor.sequence
            || self
                .last_anchor
                .is_some_and(|last| cursor.sequence <= last.cursor.sequence)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original excavation cursor regressed or crossed a closed proof owner",
            ));
        }
        Ok(())
    }
    pub(super) fn sample(
        &mut self,
        projection: &DigGoalProjection,
        evidence: &PredicateEvidence<'_>,
    ) -> Result<DigGoalProgress> {
        let anchor = projection.anchor();
        self.cursor(anchor.cursor)?;
        if anchor.tick < self.high_tick {
            return Err(error(
                ErrorCode::StaleAnchor,
                "excavation proof is below the retained authority clock",
            ));
        }
        let basis = self
            .last_anchor
            .map_or(self.cursor_floor, |last| last.cursor);
        let continuous =
            basis.sequence.checked_add(1) == Some(anchor.cursor.sequence) && !self.interrupted;
        let mut runtime = self.runtime.clone();
        if !continuous && let Some(runtime) = &mut runtime {
            runtime.observation_interrupted(self.action_id)?;
        }
        let spec = self.review.original_plan().steps[0].obligation.as_ref();
        let postconditions = evidence.evaluate(&self.postconditions)?;
        let plan_terminal = evidence.evaluate(&self.review.original_plan().terminal_condition)?;
        let obligation_terminal = spec
            .map(|spec| evidence.evaluate(&spec.terminal))
            .transpose()?;
        let failure_predicate = spec
            .and_then(|spec| spec.failure.as_ref())
            .map(|p| evidence.evaluate(p))
            .transpose()?;
        let current_goal = evidence.evaluate(&self.conjunction)?;
        if let Some(runtime) = &mut runtime {
            runtime.step_tick_with_evidence(evidence)?;
        }
        let obligation = runtime
            .as_ref()
            .and_then(|runtime| runtime.get_status(self.action_id))
            .cloned();
        let mut progress = DigGoalProgress {
            anchor,
            review_seal: self.review.seal(),
            plan_digest: self.review.original_plan().digest,
            source_digest: projection.source_digest(),
            postconditions,
            plan_terminal,
            obligation_terminal,
            failure_predicate,
            current_goal,
            obligation,
            first_satisfied_anchor: self.first_satisfied_anchor,
            receipt_linked: projection.receipt_linked(),
            excavation_condition: projection.excavation_condition(),
            excavation_proven: projection.excavation_proven(evidence)?,
            continuous,
        };
        if progress.current_goal_proven() && progress.first_satisfied_anchor.is_none() {
            progress.first_satisfied_anchor = Some(anchor);
        }
        self.runtime = runtime;
        self.first_satisfied_anchor = progress.first_satisfied_anchor;
        self.last_anchor = Some(anchor);
        self.cursor_floor = anchor.cursor;
        self.high_tick = anchor.tick;
        self.interrupted = false;
        self.latest = Some(progress.clone());
        Ok(progress)
    }
}

/// Only this module and the private durable owner may publish proof transitions.
pub(super) trait GoalPublication {
    fn preflight(&mut self, _cursor: ObservationCursor, _call: &mut Call) -> Result<()> {
        Ok(())
    }
    fn started(
        &mut self,
        _monitor: &DigGoalMonitor,
        _cursor: ObservationCursor,
        _call: &mut Call,
    ) -> Result<()> {
        Ok(())
    }
    fn captured(
        &mut self,
        _sample: &LiveSpatialCitizenObservation,
        _call: &mut Call,
    ) -> Result<()> {
        Ok(())
    }
    fn published(
        &mut self,
        _projection: &DigGoalProjection,
        _policy: &EvidencePolicy,
        _entry: &DigRecord,
        _call: &mut Call,
    ) -> Result<()> {
        Ok(())
    }
    fn failed(&mut self, _monitor: &DigGoalMonitor, _context: &OperationContext) -> Result<()> {
        Ok(())
    }
}
struct ProcessLocalPublication;
impl GoalPublication for ProcessLocalPublication {}

impl<S: EffectJournalStorage, B: EffectJournalStorage, N: DigSource> SemanticDigSession<S, B, N> {
    pub fn begin_goal_monitor(
        &mut self,
        seal: Digest32,
        context: &OperationContext,
    ) -> Result<DigGoalMonitor> {
        let mut call = Call::new(context)?;
        call.reserve(LOCAL_BYTES)?;
        let inventory = self.view(&mut call)?;
        let review = self.attached(seal)?;
        self.exact(&review)?;
        original(review.original_plan(), &call)?;
        let entry = inventory
            .entry(review.native_plan().key())
            .ok_or_else(custody)?;
        if entry.plan() != review.native_plan() {
            return Err(custody());
        }
        if entry.state() != DigState::Terminal
            || entry
                .effect()
                .is_none_or(|r| r.phase() != DigPhase::Designated)
        {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "reconcile the original designation before monitoring excavation",
            ));
        }
        authorize(review.original_plan().anchor, &call.current()?)?;
        if self.goal_floor.fenced {
            return Err(error(
                ErrorCode::StaleAnchor,
                "excavation source is fenced; replacing a monitor cannot erase consumed evidence",
            ));
        }
        let owner = Arc::new(());
        let mut monitor = DigGoalMonitor::new(review, &call.current()?, Arc::downgrade(&owner))?;
        if let Some(anchor) = self.floor.canonical {
            monitor.cursor_floor.sequence =
                monitor.cursor_floor.sequence.max(anchor.cursor.sequence);
        }
        if let Some(cursor) = self.goal_floor.cursor {
            if cursor.epoch != monitor.cursor_floor.epoch {
                return Err(custody());
            }
            monitor.cursor_floor.sequence = monitor.cursor_floor.sequence.max(cursor.sequence);
        }
        monitor.high_tick = monitor.high_tick.max(self.goal_floor.high_tick);
        self.abandon_preparation();
        self.goal_owner = Some(owner);
        Ok(monitor)
    }

    pub fn poll_original_goal<P, F, A>(
        &mut self,
        monitor: &mut DigGoalMonitor,
        cursor: ObservationCursor,
        context: &OperationContext,
        source_factory: F,
        policy: A,
    ) -> Result<DigGoalResult>
    where
        P: DigGoalSource,
        F: FnOnce(&DigBinding, &OperationContext) -> Result<P>,
        A: FnMut(&DigGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        self.poll_original_goal_with_publication(
            monitor,
            cursor,
            context,
            source_factory,
            policy,
            &mut ProcessLocalPublication,
        )
    }

    pub(super) fn poll_original_goal_with_publication<P, F, A, J>(
        &mut self,
        monitor: &mut DigGoalMonitor,
        cursor: ObservationCursor,
        context: &OperationContext,
        source_factory: F,
        mut policy: A,
        publication: &mut J,
    ) -> Result<DigGoalResult>
    where
        P: DigGoalSource,
        F: FnOnce(&DigBinding, &OperationContext) -> Result<P>,
        A: FnMut(&DigGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
        J: GoalPublication,
    {
        monitor.latest = None;
        let mut owner_matched = false;
        let mut consumed = false;
        let mut outer_call = match Call::new(context) {
            Ok(call) => call,
            Err(cause) => {
                monitor.observation_interrupted()?;
                return Err(cause);
            }
        };
        let result = (|| {
            let owner = monitor.owner.upgrade().ok_or_else(|| {
                error(
                    ErrorCode::StaleAnchor,
                    "original excavation monitor belongs to a replaced or closed owner",
                )
            })?;
            if self
                .goal_owner
                .as_ref()
                .is_none_or(|current| !Arc::ptr_eq(current, &owner))
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "original excavation monitor belongs to another session opening",
                ));
            }
            owner_matched = true;
            let call = &mut outer_call;
            call.context.anchor.tick = call
                .context
                .anchor
                .tick
                .max(monitor.high_tick)
                .max(self.goal_floor.high_tick);
            self.floor.authority(call)?;
            monitor.high_tick = monitor.high_tick.max(call.context.anchor.tick);
            authorize(monitor.review.original_plan().anchor, &call.current()?)?;
            if self.native.mode() == DigMode::Offline || self.goal_floor.fenced {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "offline or fenced dig custody cannot open a live excavation source",
                ));
            }
            monitor.cursor_floor.sequence = monitor
                .cursor_floor
                .sequence
                .max(context.anchor.cursor.sequence)
                .max(self.floor.canonical.map_or(0, |a| a.cursor.sequence))
                .max(self.goal_floor.cursor.map_or(0, |c| c.sequence));
            monitor.cursor(cursor)?;
            let required = 2 * self.view_reserve()
                + LOCAL_BYTES
                + PROJECTION_BYTES
                + SOURCE_BYTES
                + CONNECT_BYTES
                + 2 * POLICY_BYTES
                + OUTPUT_BYTES
                + super::STORE_BYTES
                + 1;
            if call.bytes < required
                || context.budget.max_entities == 0
                || u64::from(context.budget.max_output_tokens) * 4 < OUTPUT_BYTES
            {
                return Err(exhausted());
            }
            publication.preflight(cursor, call)?;
            if call.bytes < required {
                return Err(exhausted());
            }
            call.reserve(LOCAL_BYTES + OUTPUT_BYTES)?;
            let projection_budget = call.reserve(PROJECTION_BYTES)?;
            let source_budget = call.reserve(SOURCE_BYTES)?;
            let connect_budget = call.reserve(CONNECT_BYTES)?;
            let policy_budget = call.reserve(POLICY_BYTES)?;
            let final_policy_budget = call.reserve(POLICY_BYTES)?;
            let inventory = self.view(call)?;
            let review = self.attached(monitor.review.seal())?;
            self.exact(&review)?;
            if review.original_plan() != monitor.review.original_plan()
                || review.native_plan() != monitor.review.native_plan()
                || review.original_object_digest() != monitor.review.original_object_digest()
            {
                return Err(custody());
            }
            let entry = inventory
                .entry(review.native_plan().key())
                .cloned()
                .ok_or_else(custody)?;
            let record = entry.effect().filter(|record|
                entry.state() == DigState::Terminal && record.phase() == DigPhase::Designated)
                .ok_or_else(|| error(ErrorCode::EffectIndeterminate,
                    "original excavation remains indeterminate until designation reconciliation"))?;
            let designation = NativeDigDesignation::Designated {
                receipt: record.receipt().ok_or_else(custody)?,
                designated_count: record.designated_count().ok_or_else(custody)?,
            };
            publication.started(monitor, cursor, call)?;
            self.goal_floor.consume(cursor)?;
            consumed = true;
            let sample = {
                let mut source =
                    source_factory(self.native.binding(), &edge(call, &connect_budget)?)?;
                source
                    .read_excavation(review.native_plan(), &edge(call, &source_budget)?)
                    .inspect_err(|_| {
                        // A failed read may have consumed an unretained reset or
                        // higher clock. Replacing the monitor cannot hide it.
                        self.goal_floor.fenced = true;
                        monitor.fenced = true;
                    })?
            };
            publication.captured(&sample, call).inspect_err(|_| {
                self.goal_floor.fenced = true;
                monitor.fenced = true;
            })?;
            let projection = DigGoalProjection::new(
                &review,
                self.native.binding(),
                &entry,
                sample,
                cursor,
                &edge(call, &projection_budget)?,
            )
            .inspect_err(|_| {
                self.goal_floor.fenced = true;
                monitor.fenced = true;
            })?;
            self.goal_floor
                .retain(&review, &projection, record)
                .inspect_err(|_| monitor.fenced = true)?;
            monitor.high_tick = monitor.high_tick.max(projection.anchor().tick);
            self.floor.retain_tick(
                review.original_plan().anchor.cursor.epoch,
                monitor.high_tick,
            )?;
            self.floor.permit_anchor(projection.anchor())?;
            if projection.anchor().tick < call.context.anchor.tick {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "fresh excavation capture predates the current authority floor",
                ));
            }
            call.context.anchor.tick = call.context.anchor.tick.max(monitor.high_tick);
            let mut qualified = edge(call, &policy_budget)?;
            qualified.anchor = projection.anchor();
            authorize(review.original_plan().anchor, &qualified)?;
            let granted = policy(&projection, &qualified)?;
            let evidence = projection.evidence(granted.clone())?;
            let mut pending = monitor.pending_sample();
            let progress = pending.sample(&projection, &evidence)?;
            let final_inventory = self.view(call)?;
            self.exact(&review)?;
            if final_inventory.entry(review.native_plan().key()) != Some(&entry) {
                return Err(custody());
            }
            let mut qualified = edge(call, &final_policy_budget)?;
            qualified.anchor = projection.anchor();
            authorize(review.original_plan().anchor, &qualified)?;
            if policy(&projection, &qualified)? != granted {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "original excavation evidence authority changed before publication",
                ));
            }
            self.associations
                .verify(&call.reserve(super::STORE_BYTES)?)?;
            authorize(review.original_plan().anchor, &call.current()?)?;
            self.floor.retain_anchor(projection.anchor())?;
            publication.published(&projection, &granted, &entry, call)?;
            *monitor = pending;
            Ok(DigGoalResult {
                progress,
                designation,
            })
        })();
        self.abandon_preparation();
        if result.is_err() {
            if consumed {
                monitor.cursor_floor = cursor;
            }
            if owner_matched {
                monitor.high_tick = monitor.high_tick.max(self.goal_floor.high_tick);
                self.floor
                    .retain_tick(
                        monitor.review.original_plan().anchor.cursor.epoch,
                        monitor.high_tick,
                    )
                    .inspect_err(|_| monitor.fenced = true)?;
            }
            monitor.observation_interrupted()?;
            if let Ok(current) = outer_call.current() {
                if let Err(cause) = publication.failed(monitor, &current) {
                    monitor.fenced = true;
                    return Err(cause);
                }
            } else if consumed {
                // A durable Started record without its end boundary must fence
                // replay; do not renew the original wall budget to hide the gap.
                monitor.fenced = true;
                self.goal_floor.fenced = true;
            }
        }
        result
    }
}
