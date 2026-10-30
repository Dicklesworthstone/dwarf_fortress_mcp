//! Bounded, foreground proof of the exact original construction goal.
//!
//! Historical placement, current construction, original predicate truth and
//! temporal achievement remain separate. Monitoring never dispatches an effect.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use dfmcp_core::{
    ActionId, Capability, Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext,
    Result, RiskTier, StateAnchor,
};
use dfmcp_intent::{ObligationRuntime, ObligationStatus};
use dfmcp_world::{EvidencePolicy, Predicate, PredicateEvidence, PredicateTruth};

use crate::build_placement::journal::{BuildEntry, BuildMode, BuildSource, BuildState};
use crate::build_placement::rpc::BuildCancellation;
use crate::build_placement::{BuildBinding, BuildPhase, BuildRecord};
use crate::construction_plan::rpc::{ReadPermission, acquire_receipt_trusted};
use crate::construction_plan::{LinkedSample, SourceIdentity};
use crate::control_effect_journal::EffectJournalStorage;

use super::goal_projection::{FurnitureGoalProjection, MAX_GOAL_ENTITIES};
use super::{
    Call, LOCAL_BYTES, NativeFurniturePlacement, SemanticFurnitureReview, SemanticFurnitureSession,
    custody, error, exhausted, original,
};

pub(super) const PROJECTION_BYTES: u64 = 128 * 1024 * 1024;
pub(super) const POLICY_BYTES: u64 = 1024 * 1024;
pub(super) const SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const CONNECT_BYTES: u64 = 1024 * 1024;
const OUTPUT_BYTES: u64 = 8192;

/// A trusted host owns acquisition, credentials and the one-connection bracket.
/// Serialized samples alone do not establish native observation authority.
pub trait FurnitureGoalSource {
    fn read_construction(
        &mut self,
        original: &BuildRecord,
        context: &OperationContext,
    ) -> Result<LinkedSample>;
}

/// One-use fixed native acquisition. Credentials and permission callbacks are
/// host-owned, never tool arguments or serialized proof fields.
pub struct NativeFurnitureGoalSource {
    binding: BuildBinding,
    operations_token: Vec<u8>,
    build_token: Vec<u8>,
    nonce: [u8; 32],
    cancellation: BuildCancellation,
    permission: Option<ReadPermission>,
}
impl NativeFurnitureGoalSource {
    pub fn new(
        binding: BuildBinding,
        operations_token: Vec<u8>,
        build_token: Vec<u8>,
        nonce: [u8; 32],
        cancellation: BuildCancellation,
        permission: ReadPermission,
    ) -> Self {
        Self {
            binding,
            operations_token,
            build_token,
            nonce,
            cancellation,
            permission: Some(permission),
        }
    }
}
impl FurnitureGoalSource for NativeFurnitureGoalSource {
    fn read_construction(
        &mut self,
        original: &BuildRecord,
        context: &OperationContext,
    ) -> Result<LinkedSample> {
        let permission = self.permission.take().ok_or_else(|| {
            error(
                ErrorCode::Conflict,
                "original construction source is single-use; open a fresh bounded acquisition",
            )
        })?;
        acquire_receipt_trusted(
            &self.binding,
            original,
            std::mem::take(&mut self.operations_token),
            std::mem::take(&mut self.build_token),
            self.nonce,
            context,
            self.cancellation.clone(),
            permission,
        )
    }
}

pub(super) fn authorize(anchor: StateAnchor, context: &OperationContext) -> Result<()> {
    if context.anchor.fortress_id != anchor.fortress_id
        || context.anchor.cursor.epoch != anchor.cursor.epoch
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "original furniture goal cannot cross a fortress or restore epoch",
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
pub struct FurnitureGoalProgress {
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
    pub construction_condition: &'static str,
    pub construction_proven: bool,
    pub continuous: bool,
}
impl FurnitureGoalProgress {
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
pub struct FurnitureGoalResult {
    progress: FurnitureGoalProgress,
    placement: NativeFurniturePlacement,
}
impl FurnitureGoalResult {
    pub fn progress(&self) -> &FurnitureGoalProgress {
        &self.progress
    }
    pub fn placement_result(&self) -> &NativeFurniturePlacement {
        &self.placement
    }
    pub fn original_goal_proven(&self) -> bool {
        self.progress.current_goal_proven()
    }
    pub fn semantic_completion_proven(&self) -> bool {
        self.original_goal_proven()
            && self.progress.construction_proven
            && matches!(self.placement, NativeFurniturePlacement::Placed { .. })
    }
}

#[derive(Clone, Debug, Default)]
struct SelectedFloor {
    building_retired: bool,
    item_retired: bool,
    job_retired: bool,
    high_stage: i32,
}

/// Consumed source facts survive a denied policy and replacement of a monitor.
/// This is an anti-rollback floor, never a grant to use those facts as evidence.
#[derive(Clone, Debug, Default)]
pub(super) struct FurnitureSourceFloor {
    pub(super) source: Option<SourceIdentity>,
    pub(super) cursor: Option<ObservationCursor>,
    pub(super) high_tick: GameTick,
    next_job: u32,
    next_building: u32,
    next_item: u32,
    selected: BTreeMap<Digest32, SelectedFloor>,
    pub(super) fenced: bool,
}
impl FurnitureSourceFloor {
    /// Replay proves history chronologically before merging its anti-rollback
    /// floor with a possibly newer live owner. It never republishes old truth.
    pub(super) fn merge_retained(&mut self, other: &Self) -> Result<()> {
        self.fenced |= other.fenced;
        self.high_tick = self.high_tick.max(other.high_tick);
        self.next_job = self.next_job.max(other.next_job);
        self.next_building = self.next_building.max(other.next_building);
        self.next_item = self.next_item.max(other.next_item);
        if self
            .source
            .as_ref()
            .zip(other.source.as_ref())
            .is_some_and(|(a, b)| a != b)
            || self
                .cursor
                .zip(other.cursor)
                .is_some_and(|(a, b)| a.epoch != b.epoch)
        {
            self.fenced = true;
            return Err(error(
                ErrorCode::StaleAnchor,
                "retained construction history belongs to another source or restore epoch",
            ));
        }
        if self.source.is_none() {
            self.source = other.source.clone();
        }
        if let Some(cursor) = other.cursor
            && self.cursor.is_none_or(|old| old.sequence < cursor.sequence)
        {
            self.cursor = Some(cursor);
        }
        for (seal, retained) in &other.selected {
            if !self.selected.contains_key(seal) && self.selected.len() >= super::MAX_ASSOCIATIONS {
                self.fenced = true;
                return Err(exhausted());
            }
            let current = self.selected.entry(*seal).or_default();
            current.building_retired |= retained.building_retired;
            current.item_retired |= retained.item_retired;
            current.job_retired |= retained.job_retired;
            current.high_stage = current.high_stage.max(retained.high_stage);
        }
        Ok(())
    }
    pub(super) fn consume(&mut self, cursor: ObservationCursor) -> Result<()> {
        if self.fenced
            || self
                .cursor
                .is_some_and(|old| cursor.epoch != old.epoch || cursor.sequence <= old.sequence)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original construction cursor was consumed, regressed or crossed a fenced source",
            ));
        }
        self.cursor = Some(cursor);
        Ok(())
    }
    pub(super) fn retain(
        &mut self,
        review: &SemanticFurnitureReview,
        projection: &FurnitureGoalProjection,
        record: &BuildRecord,
    ) -> Result<()> {
        let assessment = projection.assessment();
        let raw = &assessment.operations;
        if self.fenced
            || self
                .source
                .as_ref()
                .is_some_and(|old| old != &assessment.source)
            || raw.jobs.tick() < self.high_tick
            || raw.jobs.next_job_id < self.next_job
            || raw.next_building_id < self.next_building
            || raw.next_item_id < self.next_item
        {
            self.fenced = true;
            return Err(error(
                ErrorCode::StaleAnchor,
                "construction capture changed source, clock or native allocation lineage",
            ));
        }
        self.source = Some(assessment.source.clone());
        self.high_tick = raw.jobs.tick();
        self.next_job = raw.jobs.next_job_id;
        self.next_building = raw.next_building_id;
        self.next_item = raw.next_item_id;
        let insertion = record.insertion().ok_or_else(custody)?;
        let building = raw
            .buildings
            .iter()
            .find(|row| row.native_id == insertion.building_id());
        let item_present = raw
            .items
            .iter()
            .any(|row| row.native_id == insertion.item_id());
        let job_present = raw
            .jobs
            .jobs
            .iter()
            .any(|row| row.native_id == insertion.job_id());
        if !self.selected.contains_key(&review.seal())
            && self.selected.len() >= super::MAX_ASSOCIATIONS
        {
            self.fenced = true;
            return Err(exhausted());
        }
        let floor = self.selected.entry(review.seal()).or_default();
        if (floor.building_retired && building.is_some())
            || (floor.item_retired && item_present)
            || (floor.job_retired && job_present)
            || building.is_some_and(|row| row.build_stage < floor.high_stage)
        {
            self.fenced = true;
            return Err(error(
                ErrorCode::StaleAnchor,
                "original construction identity reappeared or its retained stage regressed",
            ));
        }
        floor.building_retired |= building.is_none();
        floor.item_retired |= !item_present;
        floor.job_retired |= !job_present;
        if let Some(building) = building {
            floor.high_stage = building.build_stage;
        }
        Ok(())
    }
}

/// Exclusive proof owner. Reattachment preserves the original deadline and
/// resets unfinished stability; no public clone can branch around a denial.
#[derive(Debug)]
pub struct FurnitureGoalMonitor {
    pub(super) owner: Weak<()>,
    pub(super) review: SemanticFurnitureReview,
    postconditions: Predicate,
    conjunction: Predicate,
    pub(super) runtime: Option<ObligationRuntime>,
    pub(super) action_id: ActionId,
    pub(super) cursor_floor: ObservationCursor,
    pub(super) high_tick: GameTick,
    pub(super) last_anchor: Option<StateAnchor>,
    pub(super) first_satisfied_anchor: Option<StateAnchor>,
    latest: Option<FurnitureGoalProgress>,
    pub(super) interrupted: bool,
    pub(super) fenced: bool,
}
impl FurnitureGoalMonitor {
    pub(super) fn new(
        review: SemanticFurnitureReview,
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
    pub fn latest(&self) -> Option<&FurnitureGoalProgress> {
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
                "original construction cursor regressed or crossed a closed proof owner",
            ));
        }
        Ok(())
    }
    pub(super) fn sample(
        &mut self,
        projection: &FurnitureGoalProjection,
        evidence: &PredicateEvidence<'_>,
    ) -> Result<FurnitureGoalProgress> {
        let anchor = projection.anchor();
        self.cursor(anchor.cursor)?;
        if anchor.tick < self.high_tick {
            return Err(error(
                ErrorCode::StaleAnchor,
                "construction proof is below the retained authority clock",
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
        let mut progress = FurnitureGoalProgress {
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
            construction_condition: projection.assessment().condition.status,
            construction_proven: projection.construction_proven(evidence)?,
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
        _monitor: &FurnitureGoalMonitor,
        _cursor: ObservationCursor,
        _call: &mut Call,
    ) -> Result<()> {
        Ok(())
    }
    fn captured(&mut self, _sample: &LinkedSample, _call: &mut Call) -> Result<()> {
        Ok(())
    }
    fn published(
        &mut self,
        _projection: &FurnitureGoalProjection,
        _policy: &EvidencePolicy,
        _entry: &BuildEntry,
        _call: &mut Call,
    ) -> Result<()> {
        Ok(())
    }
    fn failed(
        &mut self,
        _monitor: &FurnitureGoalMonitor,
        _context: &OperationContext,
    ) -> Result<()> {
        Ok(())
    }
}
struct ProcessLocalPublication;
impl GoalPublication for ProcessLocalPublication {}

impl<S: EffectJournalStorage, B: EffectJournalStorage, N: BuildSource>
    SemanticFurnitureSession<S, B, N>
{
    pub fn begin_goal_monitor(
        &mut self,
        seal: Digest32,
        context: &OperationContext,
    ) -> Result<FurnitureGoalMonitor> {
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
        if entry.state() != BuildState::Terminal
            || entry
                .native()
                .is_none_or(|r| r.phase() != BuildPhase::Placed)
        {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "reconcile the original placement before monitoring construction",
            ));
        }
        authorize(review.original_plan().anchor, &call.current()?)?;
        if self.goal_floor.fenced {
            return Err(error(
                ErrorCode::StaleAnchor,
                "construction source is fenced; replacing a monitor cannot erase consumed evidence",
            ));
        }
        let owner = Arc::new(());
        let mut monitor =
            FurnitureGoalMonitor::new(review, &call.current()?, Arc::downgrade(&owner))?;
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
        monitor: &mut FurnitureGoalMonitor,
        cursor: ObservationCursor,
        context: &OperationContext,
        source_factory: F,
        policy: A,
    ) -> Result<FurnitureGoalResult>
    where
        P: FurnitureGoalSource,
        F: FnOnce(&BuildBinding, &OperationContext) -> Result<P>,
        A: FnMut(&FurnitureGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
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
        monitor: &mut FurnitureGoalMonitor,
        cursor: ObservationCursor,
        context: &OperationContext,
        source_factory: F,
        mut policy: A,
        publication: &mut J,
    ) -> Result<FurnitureGoalResult>
    where
        P: FurnitureGoalSource,
        F: FnOnce(&BuildBinding, &OperationContext) -> Result<P>,
        A: FnMut(&FurnitureGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
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
                    "original construction monitor belongs to a replaced or closed owner",
                )
            })?;
            if self
                .goal_owner
                .as_ref()
                .is_none_or(|current| !Arc::ptr_eq(current, &owner))
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "original construction monitor belongs to another session opening",
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
            if self.native.mode() == BuildMode::Offline || self.goal_floor.fenced {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "offline or fenced furniture custody cannot open a live construction source",
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
                || context.budget.max_entities < MAX_GOAL_ENTITIES as u32
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
            let record = entry.native().filter(|record|
                entry.state() == BuildState::Terminal && record.phase() == BuildPhase::Placed)
                .ok_or_else(|| error(ErrorCode::EffectIndeterminate,
                    "original construction remains indeterminate until placement reconciliation"))?;
            let insertion = record.insertion().ok_or_else(custody)?;
            let placement = NativeFurniturePlacement::Placed {
                native_building_id: insertion.building_id(),
                native_job_id: insertion.job_id(),
                native_item_id: insertion.item_id(),
                receipt: record.receipt(),
            };
            publication.started(monitor, cursor, call)?;
            self.goal_floor.consume(cursor)?;
            consumed = true;
            let sample = {
                let mut source =
                    source_factory(self.native.binding(), &edge(call, &connect_budget)?)?;
                source
                    .read_construction(record, &edge(call, &source_budget)?)
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
            let projection = FurnitureGoalProjection::new(
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
                    "fresh construction capture predates the current authority floor",
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
                    "original construction evidence authority changed before publication",
                ));
            }
            self.associations
                .verify(&call.reserve(super::STORE_BYTES)?)?;
            authorize(review.original_plan().anchor, &call.current()?)?;
            self.floor.retain_anchor(projection.anchor())?;
            publication.published(&projection, &granted, &entry, call)?;
            *monitor = pending;
            Ok(FurnitureGoalResult {
                progress,
                placement,
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
