//! Bounded foreground observation of the exact original work-order goal.
//! Native insertion, current goal truth, temporal achievement and physical
//! quiescence remain separate. No game effect or background worker is reachable.

use std::sync::{Arc, Weak};
use std::time::Duration;

use dfmcp_core::{
    ActionId, Capability, Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext,
    Result, RiskTier, StateAnchor,
};
use dfmcp_intent::{ObligationRuntime, ObligationStatus};
use dfmcp_world::{EvidencePolicy, Predicate, PredicateEvidence, PredicateTruth};

use crate::control_effect_journal::EffectJournalStorage;
use crate::live_jobs::JobPublication;
use crate::live_jobs_rpc::{DeadlineStream, operations::OperationsRpcClient};
use crate::live_operations::{LiveOperationsObservation, LiveOperationsState};
use crate::work_order_control::{CreationRecord, CreationState, WorkOrderSource};
use crate::work_order_progress::{
    BOOTSTRAP_BYTE_RESERVE, ProgressManifest, ProgressObservation, ProgressSource, RPC_BYTE_RESERVE,
};
use crate::work_orders::{WorkOrderEffect, WorkOrderObservation, WorkOrderState};

use super::goal_projection::{
    MAX_GOAL_EDGES, MAX_GOAL_ENTITIES, WorkOrderGoalProjection, WorkOrderPhysicalState,
};
use super::{
    Call, LOCAL_BYTES, NATIVE_BYTES, NativeOrderCreation, SemanticWorkOrderReview,
    SemanticWorkOrderSession, VIEW_COST, custody, error, exhausted, original,
};

const PROJECTION_BYTES: u64 = 128 * 1024 * 1024;
const OPERATIONS_BYTES: u64 = 16 * 1024 * 1024;
const POLICY_BYTES: u64 = 1024 * 1024;
const OUTPUT_BYTES: u64 = 8192;
const POLL_BYTES: u64 = LOCAL_BYTES
    + PROJECTION_BYTES
    + OPERATIONS_BYTES
    + 2 * BOOTSTRAP_BYTE_RESERVE
    + 2 * (VIEW_COST + NATIVE_BYTES + RPC_BYTE_RESERVE + POLICY_BYTES)
    + OUTPUT_BYTES
    + super::STORE_BYTES
    + 1;

/// Closed complete operations acquisition. The concrete implementation uses the
/// existing operations/1.3 TCP client, including its remaining socket deadline.
/// A factory owns connection setup and credentials; neither is an MCP argument.
pub trait OperationsGoalSource {
    fn read_operations(&mut self, context: &OperationContext) -> Result<LiveOperationsObservation>;
}
impl OperationsGoalSource for OperationsRpcClient<DeadlineStream> {
    fn read_operations(&mut self, context: &OperationContext) -> Result<LiveOperationsObservation> {
        self.refresh(Duration::from_millis(context.budget.max_wall_millis))
    }
}

fn authorize(original: StateAnchor, c: &OperationContext) -> Result<()> {
    if c.anchor.fortress_id != original.fortress_id
        || c.anchor.cursor.epoch != original.cursor.epoch
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "original work-order goal cannot cross a fortress or restore epoch",
        ));
    }
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
}
fn creation(
    review: &SemanticWorkOrderReview,
    record: &CreationRecord,
) -> Result<NativeOrderCreation> {
    if record.plan() != &review.native {
        return Err(custody());
    }
    let effect = WorkOrderEffect::decode(record.effect().canonical_bytes(), &review.native)?;
    if record.state() == CreationState::Created && effect.state() == WorkOrderState::Created {
        Ok(NativeOrderCreation::Inserted {
            native_order_id: effect.created_order_id().ok_or_else(custody)?,
            receipt: effect.receipt().ok_or_else(custody)?,
        })
    } else {
        Ok(NativeOrderCreation::Unverified)
    }
}

#[derive(Clone, Debug)]
pub struct WorkOrderGoalProgress {
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
    pub physical_work: WorkOrderPhysicalState,
    pub receipt_linked: bool,
    pub continuous: bool,
}
impl WorkOrderGoalProgress {
    pub fn current_goal_proven(&self) -> bool {
        self.current_goal == PredicateTruth::True
            && self.failure_predicate != Some(PredicateTruth::True)
            && self
                .obligation
                .as_ref()
                .is_none_or(|status| matches!(status, ObligationStatus::Fulfilled { .. }))
    }
}
#[derive(Clone, Debug)]
pub struct WorkOrderGoalResult {
    progress: WorkOrderGoalProgress,
    creation: NativeOrderCreation,
    native_state: CreationState,
}
impl WorkOrderGoalResult {
    pub fn progress(&self) -> &WorkOrderGoalProgress {
        &self.progress
    }
    pub fn creation_result(&self) -> &NativeOrderCreation {
        &self.creation
    }
    pub fn native_state(&self) -> CreationState {
        self.native_state
    }
    pub fn original_goal_proven(&self) -> bool {
        self.progress.current_goal_proven()
    }
    pub fn semantic_completion_proven(&self) -> bool {
        self.original_goal_proven()
            && self.progress.physical_work.is_quiescent()
            && matches!(self.creation, NativeOrderCreation::Inserted { .. })
    }
}

/// Process-local proof owner. It cannot be cloned/imported to branch around a
/// contradiction. Reattachment starts a fresh streak with the original deadline.
#[derive(Debug)]
pub struct WorkOrderGoalMonitor {
    owner: Weak<()>,
    review: SemanticWorkOrderReview,
    operations: LiveOperationsState,
    progress_manifest: Option<ProgressManifest>,
    progress_floor: Option<ProgressObservation>,
    postconditions: Predicate,
    conjunction: Predicate,
    runtime: Option<ObligationRuntime>,
    action_id: ActionId,
    cursor_floor: ObservationCursor,
    high_tick: GameTick,
    last_anchor: Option<StateAnchor>,
    first_satisfied_anchor: Option<StateAnchor>,
    latest: Option<WorkOrderGoalProgress>,
    interrupted: bool,
    fenced: bool,
}
impl WorkOrderGoalMonitor {
    fn new(
        review: SemanticWorkOrderReview,
        context: &OperationContext,
        owner: Weak<()>,
    ) -> Result<Self> {
        original(&review.original)?;
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
        let action_id = ActionId::new(review.seal.first_u128().max(1));
        let runtime = if let Some(spec) = &step.obligation {
            let mut spec = spec.clone();
            spec.terminal = conjunction.clone();
            let mut runtime = ObligationRuntime::new();
            runtime.register_obligation(action_id, spec, review.original.anchor.tick)?;
            Some(runtime)
        } else {
            None
        };
        Ok(Self {
            owner,
            cursor_floor: ObservationCursor {
                epoch: review.original.anchor.cursor.epoch,
                sequence: review
                    .original
                    .anchor
                    .cursor
                    .sequence
                    .max(context.anchor.cursor.sequence),
            },
            high_tick: review.original.anchor.tick.max(context.anchor.tick),
            review,
            operations: LiveOperationsState::default(),
            progress_manifest: None,
            progress_floor: None,
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
    pub fn original_plan(&self) -> &dfmcp_intent::PreparedPlan {
        &self.review.original
    }
    pub fn review_seal(&self) -> Digest32 {
        self.review.seal
    }
    pub fn latest(&self) -> Option<&WorkOrderGoalProgress> {
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
    fn cursor(&self, cursor: ObservationCursor) -> Result<()> {
        if self.fenced
            || cursor.epoch != self.review.original.anchor.cursor.epoch
            || cursor.sequence <= self.cursor_floor.sequence
            || self
                .last_anchor
                .is_some_and(|last| cursor.sequence <= last.cursor.sequence)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original work-order goal cursor regressed, was reused, or crossed a fenced source",
            ));
        }
        Ok(())
    }
    fn capture_floor(
        &mut self,
        manifest: &ProgressManifest,
        capture: &ProgressObservation,
    ) -> Result<()> {
        if self
            .progress_manifest
            .as_ref()
            .is_some_and(|old| old != manifest)
            || self.progress_floor.as_ref().is_some_and(|old| {
                capture.sequence() <= old.sequence()
                    || capture.tick() < old.tick()
                    || capture.next_order_id() < old.next_order_id()
                    || old.rows().first().is_some_and(|row| row.order.is_none())
                        && capture
                            .rows()
                            .first()
                            .is_some_and(|row| row.order.is_some())
                    || old
                        .rows()
                        .first()
                        .and_then(|row| row.order.as_ref())
                        .zip(capture.rows().first().and_then(|row| row.order.as_ref()))
                        .is_some_and(|(before, after)| {
                            before.recipe != after.recipe
                                || before.total != after.total
                                || after.remaining > before.remaining
                        })
            })
        {
            self.fenced = true;
            return Err(error(
                ErrorCode::StaleAnchor,
                "original-order progress changed source, allocation lineage or retained finite configuration",
            ));
        }
        self.high_tick = self.high_tick.max(GameTick(capture.tick()));
        self.progress_manifest = Some(manifest.clone());
        self.progress_floor = Some(capture.clone());
        Ok(())
    }
    fn sample(
        &mut self,
        projection: &WorkOrderGoalProjection,
        policy: &EvidencePolicy,
        evidence: &PredicateEvidence<'_>,
    ) -> Result<WorkOrderGoalProgress> {
        let anchor = projection.anchor();
        self.cursor(anchor.cursor)?;
        if anchor.tick < self.high_tick {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original-goal canonical clock predates its retained source floor",
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
        let spec = self.review.original.steps[0].obligation.as_ref();
        let postconditions = evidence.evaluate(&self.postconditions)?;
        let plan_terminal = evidence.evaluate(&self.review.original.terminal_condition)?;
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
        let mut progress = WorkOrderGoalProgress {
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
            physical_work: projection.physical(evidence, policy)?,
            receipt_linked: projection.receipt_linked(),
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

fn same_progress(before: &ProgressObservation, after: &ProgressObservation) -> bool {
    before.generation() == after.generation()
        && after.sequence() > before.sequence()
        && before.tick() == after.tick()
        && before.site() == after.site()
        && before.world_folder() == after.world_folder()
        && before.paused() == after.paused()
        && before.next_order_id() == after.next_order_id()
        && before.queue_count() == after.queue_count()
        && before.rows() == after.rows()
}

fn capture_frontier(capture: &ProgressObservation, queue: &WorkOrderObservation) -> Result<()> {
    if !capture.paused()
        || capture.tick() != queue.tick()
        || capture.fortress_id() != queue.fortress_id()
        || capture.site() != queue.site_id()
        || capture.world_folder() != queue.world_folder()
        || capture.next_order_id() != queue.next_order_id()
        || capture.queue_count() as usize != queue.order_ids().len()
        || capture
            .rows()
            .iter()
            .any(|row| row.order.is_some() != queue.order_ids().contains(&row.native_order_id))
    {
        return Err(error(
            ErrorCode::StaleAnchor,
            "selected progress does not match the current paused creation queue",
        ));
    }
    Ok(())
}

impl<S: EffectJournalStorage, B: EffectJournalStorage, N: WorkOrderSource>
    SemanticWorkOrderSession<S, B, N>
{
    pub fn begin_goal_monitor(
        &mut self,
        seal: Digest32,
        context: &OperationContext,
    ) -> Result<WorkOrderGoalMonitor> {
        let mut call = Call::new(context)?;
        call.reserve(LOCAL_BYTES)?;
        let (_, records) = self.view(&mut call)?;
        let review = self.attached(seal)?;
        self.exact(&review)?;
        if !records.iter().any(|record| record.plan() == &review.native) {
            return Err(custody());
        }
        authorize(review.original.anchor, &call.current()?)?;
        let owner = Arc::new(());
        let mut monitor =
            WorkOrderGoalMonitor::new(review, &call.current()?, Arc::downgrade(&owner))?;
        if let Some(anchor) = self.floor.canonical {
            monitor.cursor_floor.sequence =
                monitor.cursor_floor.sequence.max(anchor.cursor.sequence);
        }
        self.native.clear_selection();
        self.goal_owner = Some(owner);
        Ok(monitor)
    }

    /// Capture progress before and after one complete operations/1.3 read,
    /// bracketed by identical current creation-queue captures. Source owners
    /// close before two independent policy checks and publication. All native
    /// sources must describe the same paused frontier; no clock is advanced.
    pub fn poll_original_goal<P, O, F, G, A>(
        &mut self,
        monitor: &mut WorkOrderGoalMonitor,
        cursor: ObservationCursor,
        context: &OperationContext,
        progress_factory: F,
        operations_factory: G,
        mut policy: A,
    ) -> Result<WorkOrderGoalResult>
    where
        P: ProgressSource,
        O: OperationsGoalSource,
        F: FnOnce(&OperationContext) -> Result<P>,
        G: FnOnce(&OperationContext) -> Result<O>,
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        monitor.latest = None;
        let result = (|| {
            let owner = monitor.owner.upgrade().ok_or_else(|| {
                error(
                    ErrorCode::StaleAnchor,
                    "original-goal monitor belongs to a replaced or closed owner",
                )
            })?;
            if self
                .goal_owner
                .as_ref()
                .is_none_or(|current| !Arc::ptr_eq(current, &owner))
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "original-goal monitor belongs to another session opening",
                ));
            }
            let mut call = Call::new(context)?;
            call.context.anchor.tick = call.context.anchor.tick.max(monitor.high_tick);
            self.floor.authority(&mut call)?;
            authorize(monitor.review.original.anchor, &call.current()?)?;
            monitor.cursor_floor.sequence = monitor
                .cursor_floor
                .sequence
                .max(context.anchor.cursor.sequence)
                .max(
                    self.floor
                        .canonical
                        .map_or(0, |anchor| anchor.cursor.sequence),
                );
            monitor.cursor(cursor)?;
            if context.budget.max_entities < MAX_GOAL_ENTITIES as u32
                || u64::from(context.budget.max_output_tokens) * 4 < OUTPUT_BYTES
            {
                return Err(exhausted());
            }
            call.admit(POLL_BYTES)?;
            call.reserve(LOCAL_BYTES + PROJECTION_BYTES + OUTPUT_BYTES)?;
            let policy_budget = call.reserve(POLICY_BYTES)?;
            let final_policy_budget = call.reserve(POLICY_BYTES)?;
            let (_, records) = self.view(&mut call)?;
            let review = self.attached(monitor.review.seal)?;
            self.exact(&review)?;
            if review.original != monitor.review.original || review.native != monitor.review.native
            {
                return Err(custody());
            }
            let record = records
                .iter()
                .find(|record| record.plan() == &review.native)
                .ok_or_else(custody)?;
            let creation = creation(&review, record)?;
            let native_state = record.state();
            let source_manifest = record.manifest().clone();
            let queue = self.observe_native(&mut call)?;
            if self.native.selected_manifest(&call.current()?)? != Some(&source_manifest)
                || queue.generation() != review.native.observation().generation()
                || !queue.paused()
                || queue.tick() < monitor.high_tick.get()
            {
                monitor.fenced = true;
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "current creation queue changed original source identity or paused clock",
                ));
            }
            monitor.high_tick = monitor.high_tick.max(GameTick(queue.tick()));
            call.context.anchor.tick = monitor.high_tick;
            authorize(review.original.anchor, &call.current()?)?;
            let ids = [review.native.observation().next_order_id()];
            let mut source = progress_factory(&call.reserve(BOOTSTRAP_BYTE_RESERVE)?)?;
            let manifest = source.manifest().clone();
            if manifest.generation == 0
                || manifest.generation == u64::MAX
                || manifest.df_version != source_manifest.df_version
                || manifest.dfhack_version != source_manifest.dfhack_version
            {
                source.fence();
                monitor.fenced = true;
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "progress source software differs from original native custody",
                ));
            }
            let read = call.reserve(RPC_BYTE_RESERVE)?;
            let before = source.read(&ids, Duration::from_millis(read.budget.max_wall_millis))?;
            let before = ProgressObservation::decode(before.canonical_bytes(), &ids)?;
            if source.manifest() != &manifest || before.generation() != manifest.generation {
                source.fence();
                monitor.fenced = true;
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "progress source changed during acquisition",
                ));
            }
            capture_frontier(&before, &queue)?;
            monitor.capture_floor(&manifest, &before)?;
            call.context.anchor.tick = call.context.anchor.tick.max(monitor.high_tick);
            authorize(review.original.anchor, &call.current()?)?;
            let raw = {
                let mut source = operations_factory(&call.reserve(BOOTSTRAP_BYTE_RESERVE)?)?;
                source.read_operations(&call.reserve(OPERATIONS_BYTES)?)?
            };
            if raw
                .jobs
                .jobs
                .len()
                .saturating_add(raw.items.len())
                .saturating_add(raw.buildings.len())
                .saturating_add(2)
                > MAX_GOAL_ENTITIES
                || raw.attachments.len() > MAX_GOAL_EDGES
            {
                return Err(exhausted());
            }
            raw.validate()?;
            monitor.high_tick = monitor.high_tick.max(raw.jobs.tick());
            call.context.anchor.tick = call.context.anchor.tick.max(monitor.high_tick);
            authorize(review.original.anchor, &call.current()?)?;
            let mut operations = monitor.operations.clone();
            if operations.publish(raw)? == JobPublication::Reset {
                monitor.fenced = true;
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "original-goal operations changed incarnation or allocation lineage",
                ));
            }
            // Retain observed identities and source clocks even when a later
            // policy check denies proof. They do not themselves grant authority.
            monitor.operations = operations;
            let read = call.reserve(RPC_BYTE_RESERVE)?;
            let after = source.read(&ids, Duration::from_millis(read.budget.max_wall_millis))?;
            let after = ProgressObservation::decode(after.canonical_bytes(), &ids)?;
            if source.manifest() != &manifest || !same_progress(&before, &after) {
                source.fence();
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "selected order changed across the complete operations capture",
                ));
            }
            monitor.capture_floor(&manifest, &after)?;
            drop(source);
            let final_queue = self.observe_native(&mut call)?;
            if final_queue != queue
                || self.native.selected_manifest(&call.current()?)? != Some(&source_manifest)
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "creation queue changed across the original-goal capture",
                ));
            }
            self.native.clear_selection();
            let projection = WorkOrderGoalProjection::new(
                &review,
                &creation,
                &monitor.operations,
                (before, after, manifest),
                final_queue,
                cursor,
            )?;
            self.floor.permit_anchor(projection.anchor())?;
            if projection.anchor().tick < monitor.high_tick {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "original-goal projection is below the retained authority floor",
                ));
            }
            let mut c = call.edge(&policy_budget)?;
            c.anchor = projection.anchor();
            authorize(review.original.anchor, &c)?;
            let granted = policy(&projection, &c)?;
            let evidence = projection.evidence(granted.clone())?;
            let (_, final_records) = self.view(&mut call)?;
            self.exact(&review)?;
            if !final_records.contains(record) {
                return Err(custody());
            }
            let mut c = call.edge(&final_policy_budget)?;
            c.anchor = projection.anchor();
            authorize(review.original.anchor, &c)?;
            if policy(&projection, &c)? != granted {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "original-goal evidence policy changed before publication",
                ));
            }
            self.associations
                .verify(&call.reserve(super::STORE_BYTES)?)?;
            authorize(review.original.anchor, &call.current()?)?;
            self.floor.retain_anchor(projection.anchor())?;
            let progress = monitor.sample(&projection, &granted, &evidence)?;
            Ok(WorkOrderGoalResult {
                progress,
                creation,
                native_state,
            })
        })();
        self.native.clear_selection();
        if result.is_err() {
            monitor.observation_interrupted()?;
        }
        result
    }
}
