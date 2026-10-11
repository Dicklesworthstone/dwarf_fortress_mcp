//! Durable original-goal observations with independently authorized replay.
//!
//! The file retains candidate source evidence and original insertion context.
//! It never supplies source grants, current observations, or mutation authority.

use std::sync::Arc;

use dfmcp_core::{Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext, Result};
use dfmcp_world::EvidencePolicy;

use crate::control_effect_journal::EffectJournalStorage;
use crate::live_jobs::JobPublication;
use crate::live_operations::LiveOperationsObservation;
use crate::work_order_control::{CreationRecord, CreationState, WorkOrderSource};
use crate::work_order_progress::{ProgressManifest, ProgressObservation, ProgressSource};
use crate::work_orders::{WorkOrderEffect, WorkOrderObservation};

use super::goal_monitor::{
    GoalPublication, OperationsGoalSource, POLICY_BYTES, POLL_BYTES, PROJECTION_BYTES,
    WorkOrderGoalMonitor, WorkOrderGoalProgress, WorkOrderGoalResult, authorize, capture_frontier,
    historical_creation, operations_identity, progress_identity, same_progress,
};
use super::goal_projection::{MAX_GOAL_EDGES, MAX_GOAL_ENTITIES, WorkOrderGoalProjection};
use super::{
    Call, LOCAL_BYTES, NativeClock, SemanticWorkOrderSession, SourceFloor, custody, error,
};

mod store;
use store::{Event, MAX_EVENT_BYTES, MAX_POLL_BYTES, MAX_POLL_EVENTS, policy_fingerprint};
pub use store::{
    GoalHistoryStore, MAX_GOAL_HISTORY_BYTES, MAX_GOAL_HISTORY_EVENTS, PrivateGoalHistoryStore,
    open_private_goal_history_store,
};

/// One exact proof owner and its exclusively held history. Neither can be cloned
/// or detached to branch around a contradiction or an ambiguous publication.
pub struct DurableWorkOrderGoalMonitor<H> {
    monitor: WorkOrderGoalMonitor,
    history: GoalHistoryStore<H>,
}
impl<H: EffectJournalStorage> DurableWorkOrderGoalMonitor<H> {
    pub fn review_seal(&self) -> Digest32 {
        self.monitor.review_seal()
    }
    pub fn original_plan(&self) -> &dfmcp_intent::PreparedPlan {
        self.monitor.original_plan()
    }
    /// Always absent immediately after recovery; historical samples are not fresh.
    pub fn latest(&self) -> Option<&WorkOrderGoalProgress> {
        self.monitor.latest()
    }
    pub fn first_satisfied_anchor(&self) -> Option<dfmcp_core::StateAnchor> {
        self.monitor.first_satisfied_anchor()
    }
    pub fn last_observation_anchor(&self) -> Option<dfmcp_core::StateAnchor> {
        self.monitor.last_observation_anchor()
    }
    pub fn is_fenced(&self) -> bool {
        self.monitor.is_fenced() || self.history.is_fenced() || self.history.source_fenced()
    }
    pub fn history_store(&self) -> &GoalHistoryStore<H> {
        &self.history
    }
    pub fn history_event_count(&self) -> usize {
        self.history.event_count()
    }
    pub fn history_byte_len(&self) -> usize {
        self.history.byte_len()
    }
    pub fn high_cursor(&self) -> ObservationCursor {
        self.monitor.cursor_floor
    }
    pub fn high_tick(&self) -> GameTick {
        self.monitor.high_tick.max(self.history.high_tick())
    }
    /// Reauthorized historical outcome, separate from current physical work.
    pub fn historical_obligation_status(&self) -> Option<&dfmcp_intent::ObligationStatus> {
        self.monitor
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.get_status(self.monitor.action_id))
    }

    /// Persist a known observation gap without reading or changing native work.
    /// An ambiguous write fences this owner until its history is reopened.
    pub fn observation_interrupted(&mut self, context: &OperationContext) -> Result<()> {
        self.monitor.observation_interrupted()?;
        let mut current = context.clone();
        current.anchor.tick = current.anchor.tick.max(self.high_tick());
        self.monitor.high_tick = current.anchor.tick;
        let result = self.history.append(
            Event::Interrupted {
                tick: current.anchor.tick,
                fenced: self.monitor.fenced,
            },
            &current,
        );
        if result.is_err() {
            self.monitor.fenced = true;
        }
        result
    }
}

struct HistoryPublication<'a, H> {
    history: &'a mut GoalHistoryStore<H>,
    budget: Option<Call>,
    began: bool,
}
impl<H: EffectJournalStorage> HistoryPublication<'_, H> {
    fn append(&mut self, event: Event, current: &OperationContext) -> Result<()> {
        let budget = self.budget.as_mut().ok_or_else(custody)?;
        // Retain the original absolute deadline; a callback cannot replenish it.
        budget.context.anchor.tick = budget
            .context
            .anchor
            .tick
            .max(current.anchor.tick)
            .max(self.history.high_tick());
        let cost =
            LOCAL_BYTES + 6 * (self.history.byte_len() + MAX_EVENT_BYTES + 128) as u64 + 8192;
        let context = budget.reserve(cost)?;
        self.history.append(event, &context)
    }
}
impl<H: EffectJournalStorage> GoalPublication for HistoryPublication<'_, H> {
    fn preflight(&mut self, cursor: ObservationCursor, call: &mut Call) -> Result<()> {
        self.history.preflight_poll(cursor)?;
        if self.history.event_count() + MAX_POLL_EVENTS
            > call.current()?.budget.max_entities as usize
        {
            return Err(super::exhausted());
        }
        // Pay for every possible acquisition event and final interruption before
        // the first native contact. Full history verification is also bounded.
        let size = self.history.byte_len() + MAX_POLL_BYTES + MAX_EVENT_BYTES + 128;
        let verify = LOCAL_BYTES + self.history.byte_len() as u64 + 8192;
        let cost = verify + MAX_POLL_EVENTS as u64 * (LOCAL_BYTES + 6 * size as u64 + 8192) + 1;
        call.admit(POLL_BYTES + cost)?;
        let mut budget = Call::new(&call.reserve(cost)?)?;
        self.history.verify(&budget.reserve(verify)?)?;
        self.budget = Some(budget);
        Ok(())
    }
    fn started(
        &mut self,
        monitor: &WorkOrderGoalMonitor,
        cursor: ObservationCursor,
        call: &mut Call,
    ) -> Result<()> {
        self.began = true;
        self.append(
            Event::Started {
                cursor,
                tick: monitor.high_tick.max(call.current()?.anchor.tick),
                interrupted: monitor.interrupted,
            },
            &call.current()?,
        )
    }
    fn queue_captured(&mut self, queue: &WorkOrderObservation, call: &mut Call) -> Result<()> {
        self.append(
            Event::Queue {
                capture: queue.clone(),
            },
            &call.current()?,
        )
    }
    fn progress_captured(
        &mut self,
        manifest: &ProgressManifest,
        capture: &ProgressObservation,
        call: &mut Call,
    ) -> Result<()> {
        self.append(
            Event::Progress {
                manifest: manifest.clone(),
                capture: capture.clone(),
            },
            &call.current()?,
        )
    }
    fn operations_captured(
        &mut self,
        capture: &LiveOperationsObservation,
        call: &mut Call,
    ) -> Result<()> {
        self.append(Event::Operations(capture.clone()), &call.current()?)
    }
    fn published(
        &mut self,
        projection: &WorkOrderGoalProjection,
        policy: &EvidencePolicy,
        record: &CreationRecord,
        call: &mut Call,
    ) -> Result<()> {
        self.append(
            Event::Published {
                fingerprint: policy_fingerprint(projection, policy)?,
                creation_state: record.state(),
                effect: record.effect().clone(),
            },
            &call.current()?,
        )
    }
    fn failed(&mut self, monitor: &WorkOrderGoalMonitor, context: &OperationContext) -> Result<()> {
        if self.history.is_fenced() {
            return Err(custody());
        }
        if !self.began {
            return Ok(());
        }
        self.append(
            Event::Interrupted {
                tick: monitor.high_tick.max(self.history.high_tick()),
                fenced: monitor.fenced,
            },
            context,
        )
    }
}

struct Acquisition {
    cursor: ObservationCursor,
    queue: Option<WorkOrderObservation>,
    before: Option<(ProgressManifest, ProgressObservation)>,
    after: Option<(ProgressManifest, ProgressObservation)>,
    final_queue: Option<WorkOrderObservation>,
    operations: bool,
    invalid: bool,
}
impl Acquisition {
    fn new(cursor: ObservationCursor) -> Self {
        Self {
            cursor,
            queue: None,
            before: None,
            after: None,
            final_queue: None,
            operations: false,
            invalid: false,
        }
    }

    fn projection(
        self,
        monitor: &WorkOrderGoalMonitor,
        state: CreationState,
        effect: &WorkOrderEffect,
    ) -> Result<WorkOrderGoalProjection> {
        if self.invalid || !self.operations || monitor.fenced {
            return Err(custody());
        }
        let queue = self.queue.ok_or_else(custody)?;
        let final_queue = self.final_queue.ok_or_else(custody)?;
        let (manifest, before) = self.before.ok_or_else(custody)?;
        let (after_manifest, after) = self.after.ok_or_else(custody)?;
        if queue != final_queue || manifest != after_manifest || !same_progress(&before, &after) {
            return Err(custody());
        }
        capture_frontier(&before, &queue)?;
        capture_frontier(&after, &queue)?;
        let creation = historical_creation(&monitor.review, state, effect)?;
        WorkOrderGoalProjection::new(
            &monitor.review,
            &creation,
            &monitor.operations,
            (before, after, manifest),
            final_queue,
            self.cursor,
        )
    }
}

fn historical_record(
    record: &CreationRecord,
    state: CreationState,
    effect: &WorkOrderEffect,
) -> Result<()> {
    // A historical successful insertion must remain the exact terminal receipt
    // in the independent creation journal. Earlier unresolved samples remain
    // unresolved even when this record was reconciled after their publication.
    if matches!(
        state,
        CreationState::Created | CreationState::Refused | CreationState::CancelledBeforeDispatch
    ) && (state != record.state() || effect != record.effect())
    {
        return Err(custody());
    }
    Ok(())
}

impl<S: EffectJournalStorage, B: EffectJournalStorage, N: WorkOrderSource>
    SemanticWorkOrderSession<S, B, N>
{
    /// Recover one exact original goal without opening a native source or
    /// replaying insertion. Each Published sample must receive its exact source
    /// policy twice, independently, at the current maximum authority tick.
    ///
    /// Restart breaks unfinished stability and never renews the original
    /// deadline. An incomplete acquisition with no durable completion boundary
    /// fences fresh polling because an unrecorded source reset cannot be ruled out.
    pub fn begin_durable_goal_monitor<H, A>(
        &mut self,
        mut history: GoalHistoryStore<H>,
        seal: Digest32,
        context: &OperationContext,
        mut authorize_history: A,
    ) -> Result<DurableWorkOrderGoalMonitor<H>>
    where
        H: EffectJournalStorage,
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        let mut call = Call::new(context)?;
        call.context.anchor.tick = call.context.anchor.tick.max(history.high_tick());
        self.floor.authority(&mut call)?;
        let verify_cost = LOCAL_BYTES + history.byte_len() as u64 + 8192;
        history.verify(&call.reserve(verify_cost)?)?;
        let (summary, records) = self.view(&mut call)?;
        let review = self.attached(seal)?;
        self.exact(&review)?;
        history.binding(summary.journal_id, &review)?;
        // Raw history can retain a later authority clock even if independent
        // historical policy subsequently refuses proof recovery. Replacing a
        // monitor must not make an expired earlier grant usable again.
        self.floor
            .retain_tick(review.original.anchor.cursor.epoch, history.high_tick())?;
        self.floor.authority(&mut call)?;
        let record = records
            .iter()
            .find(|record| record.plan() == &review.native)
            .ok_or_else(custody)?;
        authorize(review.original.anchor, &call.current()?)?;

        let owner = Arc::new(());
        let mut historical_context = call.current()?;
        // Constructor input only. All I/O and policy authority below use `call`,
        // whose tick already includes current, journal and complete history floors.
        historical_context.anchor = review.original.anchor;
        let mut monitor =
            WorkOrderGoalMonitor::new(review.clone(), &historical_context, Arc::downgrade(&owner))?;
        let mut retained = self.floor.clone();
        let mut replay_floor = SourceFloor::default();
        replay_floor.retain_anchor(review.original.anchor)?;
        replay_floor.retain_native(NativeClock::observed(
            review.native.observation(),
            review.original.anchor.cursor.epoch,
        ))?;
        let mut pending: Option<Acquisition> = None;
        for event in history.events() {
            call.current()?;
            match event {
                Event::Started {
                    cursor,
                    tick,
                    interrupted,
                } => {
                    if let Some(prior) = pending.take() {
                        monitor.cursor_floor = prior.cursor;
                        monitor.fenced = true;
                    }
                    if *interrupted || monitor.fenced {
                        monitor.observation_interrupted()?;
                    }
                    if !monitor.fenced {
                        monitor.cursor(*cursor)?;
                    }
                    monitor.high_tick = monitor.high_tick.max(*tick);
                    pending = Some(Acquisition::new(*cursor));
                }
                Event::Queue { capture } => {
                    let sample = pending.as_mut().ok_or_else(custody)?;
                    monitor.high_tick = monitor.high_tick.max(GameTick(capture.tick()));
                    let clock = NativeClock::observed(capture, review.original.anchor.cursor.epoch);
                    let identity = capture.fortress_id() == review.original.anchor.fortress_id
                        && capture.generation() == review.native.observation().generation()
                        && capture.site_id() == review.native.observation().site_id()
                        && capture.world_folder() == review.native.observation().world_folder();
                    if !identity || replay_floor.permit_native(clock).is_err() {
                        monitor.fenced = true;
                        sample.invalid = true;
                    } else {
                        replay_floor.retain_native(clock)?;
                        retained.retain_native(clock)?;
                    }
                    if !capture.paused() {
                        sample.invalid = true;
                    }
                    if sample.queue.is_none() {
                        sample.queue = Some(capture.clone());
                    } else {
                        sample.final_queue = Some(capture.clone());
                    }
                }
                Event::Progress { manifest, capture } => {
                    let sample = pending.as_mut().ok_or_else(custody)?;
                    monitor.high_tick = monitor.high_tick.max(GameTick(capture.tick()));
                    if progress_identity(&review, record.manifest(), manifest, capture).is_err() {
                        monitor.fenced = true;
                        sample.invalid = true;
                    } else if monitor.capture_floor(manifest, capture).is_err() {
                        sample.invalid = true;
                    }
                    if sample.before.is_none() {
                        sample.before = Some((manifest.clone(), capture.clone()));
                    } else {
                        sample.after = Some((manifest.clone(), capture.clone()));
                    }
                }
                Event::Operations(raw) => {
                    let sample = pending.as_mut().ok_or_else(custody)?;
                    // Replay every capture through one persistent operations
                    // owner, including captures whose later publication failed.
                    call.reserve(PROJECTION_BYTES)?;
                    monitor.high_tick = monitor.high_tick.max(raw.jobs.tick());
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
                        return Err(custody());
                    }
                    let mut operations = monitor.operations.clone();
                    let publication = operations_identity(&review, record.manifest(), raw)
                        .and_then(|()| operations.publish(raw.clone()));
                    match publication {
                        Ok(JobPublication::Reset) | Err(_) => {
                            monitor.fenced = true;
                            sample.invalid = true;
                        }
                        Ok(_) => monitor.operations = operations,
                    }
                    sample.operations = true;
                }
                Event::Published {
                    fingerprint,
                    creation_state,
                    effect,
                } => {
                    historical_record(record, *creation_state, effect)?;
                    let projection = pending.take().ok_or_else(custody)?.projection(
                        &monitor,
                        *creation_state,
                        effect,
                    )?;
                    replay_floor.permit_anchor(projection.anchor())?;
                    let policy_budget = call.reserve(POLICY_BYTES)?;
                    let final_policy_budget = call.reserve(POLICY_BYTES)?;
                    let current = call.edge(&policy_budget)?;
                    authorize(review.original.anchor, &current)?;
                    let policy = authorize_history(&projection, &current)?;
                    if policy_fingerprint(&projection, &policy)? != *fingerprint {
                        return Err(error(
                            ErrorCode::CapabilityDenied,
                            "historical work-order source policy is unavailable or changed",
                        ));
                    }
                    let evidence = projection.evidence(policy.clone())?;
                    let mut candidate = monitor.pending_sample();
                    candidate.sample(&projection, &policy, &evidence)?;
                    let current = call.edge(&final_policy_budget)?;
                    authorize(review.original.anchor, &current)?;
                    if authorize_history(&projection, &current)? != policy {
                        return Err(error(
                            ErrorCode::CapabilityDenied,
                            "historical work-order policy changed before recovery publication",
                        ));
                    }
                    replay_floor.retain_anchor(projection.anchor())?;
                    retained.retain_anchor(projection.anchor())?;
                    monitor = candidate;
                }
                Event::Interrupted { tick, fenced } => {
                    if let Some(prior) = pending.take() {
                        monitor.cursor_floor = prior.cursor;
                    }
                    monitor.observation_interrupted()?;
                    monitor.high_tick = monitor.high_tick.max(*tick);
                    monitor.fenced |= *fenced;
                }
            }
        }
        if pending.is_some() {
            monitor.fenced = true;
        }
        monitor.observation_interrupted()?;
        monitor.cursor_floor.sequence = monitor
            .cursor_floor
            .sequence
            .max(history.high_cursor().sequence)
            .max(context.anchor.cursor.sequence)
            .max(
                retained
                    .canonical
                    .map_or(0, |anchor| anchor.cursor.sequence),
            );
        monitor.high_tick = monitor
            .high_tick
            .max(call.current()?.anchor.tick)
            .max(history.high_tick());
        retained.retain_tick(review.original.anchor.cursor.epoch, monitor.high_tick)?;
        monitor.fenced |= history.source_fenced();
        history.verify(&call.reserve(verify_cost)?)?;
        let (summary, final_records) = self.view(&mut call)?;
        self.exact(&review)?;
        history.binding(summary.journal_id, &review)?;
        if !final_records.contains(record) {
            return Err(custody());
        }
        if let Some(anchor) = self.floor.canonical {
            retained.retain_anchor(anchor)?;
            monitor.cursor_floor.sequence =
                monitor.cursor_floor.sequence.max(anchor.cursor.sequence);
        }
        if let Some(clock) = self.floor.native {
            retained.retain_native(clock)?;
        }
        if let Some((epoch, tick)) = self.floor.authority_tick {
            retained.retain_tick(epoch, tick)?;
            monitor.high_tick = monitor.high_tick.max(tick);
        }
        authorize(review.original.anchor, &call.current()?)?;
        self.native.clear_selection();
        self.floor = retained;
        self.goal_owner = Some(owner);
        Ok(DurableWorkOrderGoalMonitor { monitor, history })
    }

    /// One explicit fresh sample; sources close before proof publication. Read
    /// intent and all successful source captures are synced in acquisition order.
    /// The exact historical insertion context and policy marker are durable
    /// before a current result is exposed. No native effect is reachable.
    pub fn poll_original_goal_durable<H, P, O, F, G, A>(
        &mut self,
        monitor: &mut DurableWorkOrderGoalMonitor<H>,
        cursor: ObservationCursor,
        context: &OperationContext,
        progress_factory: F,
        operations_factory: G,
        policy: A,
    ) -> Result<WorkOrderGoalResult>
    where
        H: EffectJournalStorage,
        P: ProgressSource,
        O: OperationsGoalSource,
        F: FnOnce(&OperationContext) -> Result<P>,
        G: FnOnce(&OperationContext) -> Result<O>,
        A: FnMut(&WorkOrderGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        let mut publication = HistoryPublication {
            history: &mut monitor.history,
            budget: None,
            began: false,
        };
        let result = self.poll_original_goal_with_publication(
            &mut monitor.monitor,
            cursor,
            context,
            (progress_factory, operations_factory),
            policy,
            &mut publication,
        );
        monitor.monitor.cursor_floor.sequence = monitor
            .monitor
            .cursor_floor
            .sequence
            .max(monitor.history.high_cursor().sequence);
        result
    }
}
