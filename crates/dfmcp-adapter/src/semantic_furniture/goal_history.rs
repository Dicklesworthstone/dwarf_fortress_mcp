//! Durable original construction proof, with independent historical authority.
//!
//! The exact original receipt and bounded raw samples remain custody evidence.
//! Reopening independently reauthorizes each historical publication; stored
//! policy fingerprints cannot create grants, fresh observations or native work.

use std::sync::Arc;

use dfmcp_core::{Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext, Result};
use dfmcp_world::EvidencePolicy;

use crate::build_placement::journal::{BuildEntry, BuildSource, BuildState};
use crate::build_placement::{BuildBinding, BuildPhase, BuildRecord, MAX_RECORD_BYTES};
use crate::construction_plan::LinkedSample;
use crate::control_effect_journal::EffectJournalStorage;

use super::goal_monitor::{
    FurnitureGoalMonitor, FurnitureGoalProgress, FurnitureGoalResult, FurnitureGoalSource,
    FurnitureSourceFloor, GoalPublication, POLICY_BYTES, PROJECTION_BYTES, authorize, edge,
};
use super::goal_projection::FurnitureGoalProjection;
use super::{Call, LOCAL_BYTES, SemanticFurnitureSession, custody, error, exhausted, original};

mod store;
use store::{Event, MAX_EVENT_BYTES, MAX_POLL_EVENTS, policy_fingerprint};
pub use store::{
    GoalHistoryStore, MAX_GOAL_HISTORY_BYTES, MAX_GOAL_HISTORY_EVENTS, PrivateGoalHistoryStore,
    open_private_goal_history_store,
};

/// One exclusive monitor and its history. Neither is publicly cloneable or
/// separable to branch around a consumed contradiction or ambiguous write.
pub struct DurableFurnitureGoalMonitor<H> {
    monitor: FurnitureGoalMonitor,
    history: GoalHistoryStore<H>,
}
impl<H: EffectJournalStorage> DurableFurnitureGoalMonitor<H> {
    pub fn review_seal(&self) -> Digest32 {
        self.monitor.review_seal()
    }
    pub fn original_plan(&self) -> &dfmcp_intent::PreparedPlan {
        self.monitor.original_plan()
    }
    /// Recovery always clears current progress; historical samples are not fresh.
    pub fn latest(&self) -> Option<&FurnitureGoalProgress> {
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
    /// Recheck exact current storage custody before exposing cached history.
    /// This performs no source acquisition, policy replay or mutation.
    pub fn verify_history(&mut self, context: &OperationContext) -> Result<()> {
        let mut current = context.clone();
        current.anchor.tick = current.anchor.tick.max(self.high_tick());
        let result = self.history.verify(&current);
        if result.is_err() {
            self.monitor.observation_interrupted()?;
            self.monitor.fenced |= self.history.is_fenced();
        }
        result
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
    /// Independently reauthorized historical outcome, never current usability.
    pub fn historical_obligation_status(&self) -> Option<&dfmcp_intent::ObligationStatus> {
        self.monitor
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.get_status(self.monitor.action_id))
    }
    /// Persist a known observation gap without opening a native source. An
    /// ambiguous append fences this owner until complete verified reopening.
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
        // Context binding refreshes authority/deadline only; the separately
        // prepaid history allowance cannot be replenished by a callback.
        budget.boundary(current)?;
        budget.context.anchor.tick = budget.context.anchor.tick.max(self.history.high_tick());
        let cost =
            LOCAL_BYTES + 6 * (self.history.byte_len() + event.maximum_bytes() + 128) as u64 + 8192;
        self.history.append(event, &budget.reserve(cost)?)
    }
}
impl<H: EffectJournalStorage> GoalPublication for HistoryPublication<'_, H> {
    fn preflight(&mut self, cursor: ObservationCursor, call: &mut Call) -> Result<()> {
        self.history.preflight_poll(cursor)?;
        if self.history.event_count() + MAX_POLL_EVENTS
            > call.current()?.budget.max_entities as usize
        {
            return Err(exhausted());
        }
        let verify = LOCAL_BYTES + self.history.byte_len() as u64 + 8192;
        // Reserve each successive worst-case prefix, including the final
        // interruption. A large capture is paid once as a new body, and every
        // later complete-byte verification is still explicitly covered.
        let mut prefix = self.history.byte_len();
        let mut cost = verify + 1;
        for body in [26, MAX_EVENT_BYTES, 39 + MAX_RECORD_BYTES, 10] {
            prefix = prefix.checked_add(body + 128).ok_or_else(exhausted)?;
            cost = cost
                .checked_add(LOCAL_BYTES + 6 * prefix as u64 + 8192)
                .ok_or_else(exhausted)?;
        }
        let mut budget = Call::new(&call.reserve(cost)?)?;
        self.history.verify(&budget.reserve(verify)?)?;
        self.budget = Some(budget);
        Ok(())
    }
    fn started(
        &mut self,
        monitor: &FurnitureGoalMonitor,
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
    fn captured(&mut self, sample: &LinkedSample, call: &mut Call) -> Result<()> {
        self.append(Event::Captured(Box::new(sample.clone())), &call.current()?)
    }
    fn published(
        &mut self,
        projection: &FurnitureGoalProjection,
        policy: &EvidencePolicy,
        entry: &BuildEntry,
        call: &mut Call,
    ) -> Result<()> {
        let receipt = entry
            .native()
            .filter(|receipt| {
                entry.state() == BuildState::Terminal && receipt.phase() == BuildPhase::Placed
            })
            .ok_or_else(custody)?;
        self.append(
            Event::Published {
                fingerprint: policy_fingerprint(projection, policy)?,
                receipt: Box::new(receipt.clone()),
                dispatch_started: entry.dispatch_started(),
                cancel_requested: entry.cancel_requested(),
            },
            &call.current()?,
        )
    }
    fn failed(&mut self, monitor: &FurnitureGoalMonitor, context: &OperationContext) -> Result<()> {
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
    projection: Option<FurnitureGoalProjection>,
    invalid: bool,
}
fn historical_receipt(
    entry: &BuildEntry,
    record: &BuildRecord,
    dispatched: bool,
    cancelled: bool,
) -> Result<()> {
    if entry.state() != BuildState::Terminal
        || record.phase() != BuildPhase::Placed
        || entry.native() != Some(record)
        || entry.dispatch_started() != dispatched
        || entry.cancel_requested() != cancelled
    {
        return Err(custody());
    }
    Ok(())
}

impl<S: EffectJournalStorage, B: EffectJournalStorage, N: BuildSource>
    SemanticFurnitureSession<S, B, N>
{
    /// Recover the exact original goal without native contact or placement
    /// replay. Every historical Published sample needs the same independently
    /// issued policy twice, under the current maximum retained authority tick.
    ///
    /// Restart breaks unfinished stability and clears current progress. It
    /// preserves the original deadline, cadence floor and independently
    /// revalidated first achievement. Any unfinished acquisition fences further
    /// sampling because an unrecorded source change cannot be excluded.
    pub fn begin_goal_monitor_durable<H, A>(
        &mut self,
        seal: Digest32,
        mut history: GoalHistoryStore<H>,
        context: &OperationContext,
        mut authorize_history: A,
    ) -> Result<DurableFurnitureGoalMonitor<H>>
    where
        H: EffectJournalStorage,
        A: FnMut(&FurnitureGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        let mut call = Call::new(context)?;
        call.context.anchor.tick = call
            .context
            .anchor
            .tick
            .max(history.high_tick())
            .max(self.goal_floor.high_tick);
        self.floor.authority(&mut call)?;
        let verify_cost = LOCAL_BYTES + history.byte_len() as u64 + 8192;
        history.verify(&call.reserve(verify_cost)?)?;
        let inventory = self.view(&mut call)?;
        let review = self.attached(seal)?;
        self.exact(&review)?;
        original(review.original_plan(), &call)?;
        history.binding(inventory.journal_id, &review)?;
        let entry = inventory
            .entry(review.native_plan().key())
            .cloned()
            .ok_or_else(custody)?;
        let record = entry
            .native()
            .filter(|record| {
                entry.state() == BuildState::Terminal && record.phase() == BuildPhase::Placed
            })
            .ok_or_else(|| {
                error(
                    ErrorCode::EffectIndeterminate,
                    "reconcile original placement before recovering construction proof",
                )
            })?;
        if entry.plan() != review.native_plan() {
            return Err(custody());
        }
        let binding: BuildBinding = self.native.binding().clone();

        // Raw custody is negative evidence: retain its authority floor even if
        // subsequent independent historical policy denies recovery.
        let epoch = review.original_plan().anchor.cursor.epoch;
        self.floor.retain_tick(epoch, history.high_tick())?;
        self.goal_floor.high_tick = self.goal_floor.high_tick.max(history.high_tick());
        self.floor.authority(&mut call)?;
        authorize(review.original_plan().anchor, &call.current()?)?;
        let owner = Arc::new(());
        let mut historical_context = call.current()?;
        historical_context.anchor = review.original_plan().anchor;
        let mut monitor =
            FurnitureGoalMonitor::new(review.clone(), &historical_context, Arc::downgrade(&owner))?;
        let mut replay = FurnitureSourceFloor::default();
        replay.high_tick = review.original_plan().anchor.tick;
        let mut pending: Option<Acquisition> = None;

        for event in history.events() {
            call.current()?;
            match event {
                Event::Started {
                    cursor,
                    tick,
                    interrupted,
                } => {
                    if let Some(old) = pending.take() {
                        monitor.cursor_floor = old.cursor;
                        monitor.fenced = true;
                        replay.fenced = true;
                    }
                    if *interrupted || monitor.fenced {
                        monitor.observation_interrupted()?;
                    }
                    if !monitor.fenced {
                        monitor.cursor(*cursor)?;
                        replay.consume(*cursor)?;
                    } else {
                        replay.cursor = Some(*cursor);
                    }
                    monitor.high_tick = monitor.high_tick.max(*tick);
                    replay.high_tick = replay.high_tick.max(*tick);
                    pending = Some(Acquisition {
                        cursor: *cursor,
                        projection: None,
                        invalid: monitor.fenced,
                    });
                }
                Event::Captured(sample) => {
                    let acquisition = pending.as_mut().ok_or_else(custody)?;
                    let projection_budget = call.reserve(PROJECTION_BYTES)?;
                    let projection = FurnitureGoalProjection::new(
                        &review,
                        &binding,
                        &entry,
                        (**sample).clone(),
                        acquisition.cursor,
                        &edge(&call, &projection_budget)?,
                    );
                    match projection {
                        Ok(projection) => {
                            monitor.high_tick = monitor.high_tick.max(projection.anchor().tick);
                            if replay.retain(&review, &projection, record).is_err() {
                                monitor.fenced = true;
                                acquisition.invalid = true;
                            }
                            acquisition.projection = Some(projection);
                        }
                        Err(_) => {
                            monitor.fenced = true;
                            replay.fenced = true;
                            acquisition.invalid = true;
                        }
                    }
                }
                Event::Published {
                    fingerprint,
                    receipt,
                    dispatch_started,
                    cancel_requested,
                } => {
                    historical_receipt(&entry, receipt, *dispatch_started, *cancel_requested)?;
                    let acquisition = pending.take().ok_or_else(custody)?;
                    if acquisition.invalid || monitor.fenced {
                        return Err(custody());
                    }
                    let projection = acquisition.projection.ok_or_else(custody)?;
                    let policy_budget = call.reserve(POLICY_BYTES)?;
                    let final_policy_budget = call.reserve(POLICY_BYTES)?;
                    let current = edge(&call, &policy_budget)?;
                    authorize(review.original_plan().anchor, &current)?;
                    let policy = authorize_history(&projection, &current)?;
                    if policy_fingerprint(&projection, &policy)? != *fingerprint {
                        return Err(error(
                            ErrorCode::CapabilityDenied,
                            "historical construction policy is unavailable or changed",
                        ));
                    }
                    let evidence = projection.evidence(policy.clone())?;
                    let mut candidate = monitor.pending_sample();
                    candidate.sample(&projection, &evidence)?;
                    let current = edge(&call, &final_policy_budget)?;
                    authorize(review.original_plan().anchor, &current)?;
                    if authorize_history(&projection, &current)? != policy {
                        return Err(error(
                            ErrorCode::CapabilityDenied,
                            "historical construction policy changed before recovery publication",
                        ));
                    }
                    self.floor.retain_anchor(projection.anchor())?;
                    monitor = candidate;
                }
                Event::Interrupted { tick, fenced } => {
                    if let Some(old) = pending.take() {
                        monitor.cursor_floor = old.cursor;
                    }
                    monitor.observation_interrupted()?;
                    monitor.high_tick = monitor.high_tick.max(*tick);
                    monitor.fenced |= *fenced;
                    replay.high_tick = replay.high_tick.max(*tick);
                    replay.fenced |= *fenced;
                }
            }
            // Every acquisition consumes floors before its later source policy
            // can fail. Repeated recovery cannot reset a known retirement.
            self.goal_floor.merge_retained(&replay)?;
            self.floor
                .retain_tick(epoch, monitor.high_tick.max(replay.high_tick))?;
        }
        if pending.is_some() {
            monitor.fenced = true;
            replay.fenced = true;
        }
        replay.cursor = Some(history.high_cursor());
        replay.high_tick = replay.high_tick.max(history.high_tick());
        replay.fenced |= history.source_fenced();
        self.goal_floor.merge_retained(&replay)?;
        monitor.observation_interrupted()?;
        monitor.cursor_floor.sequence = monitor
            .cursor_floor
            .sequence
            .max(history.high_cursor().sequence)
            .max(context.anchor.cursor.sequence)
            .max(
                self.floor
                    .canonical
                    .map_or(0, |anchor| anchor.cursor.sequence),
            )
            .max(self.goal_floor.cursor.map_or(0, |cursor| cursor.sequence));
        monitor.high_tick = monitor
            .high_tick
            .max(history.high_tick())
            .max(self.goal_floor.high_tick)
            .max(call.current()?.anchor.tick);
        monitor.fenced |= self.goal_floor.fenced || history.source_fenced();
        self.floor.retain_tick(epoch, monitor.high_tick)?;
        call.context.anchor.tick = call.context.anchor.tick.max(monitor.high_tick);
        history.verify(&call.reserve(verify_cost)?)?;
        let final_inventory = self.view(&mut call)?;
        self.exact(&review)?;
        history.binding(final_inventory.journal_id, &review)?;
        if final_inventory.entry(review.native_plan().key()) != Some(&entry) {
            return Err(custody());
        }
        authorize(review.original_plan().anchor, &call.current()?)?;
        self.abandon_preparation();
        self.goal_owner = Some(owner);
        Ok(DurableFurnitureGoalMonitor { monitor, history })
    }

    /// One explicit foreground sample. Read intent and complete linked capture
    /// are durable before policy; the exact receipt/policy marker is durable
    /// before progress is exposed. All native sources close before publication.
    pub fn poll_original_goal_durable<H, P, F, A>(
        &mut self,
        monitor: &mut DurableFurnitureGoalMonitor<H>,
        cursor: ObservationCursor,
        context: &OperationContext,
        source_factory: F,
        policy: A,
    ) -> Result<FurnitureGoalResult>
    where
        H: EffectJournalStorage,
        P: FurnitureGoalSource,
        F: FnOnce(&BuildBinding, &OperationContext) -> Result<P>,
        A: FnMut(&FurnitureGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
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
            source_factory,
            policy,
            &mut publication,
        );
        monitor.monitor.cursor_floor.sequence = monitor
            .monitor
            .cursor_floor
            .sequence
            .max(monitor.history.high_cursor().sequence);
        if monitor.history.is_fenced() || monitor.history.source_fenced() {
            monitor.monitor.fenced = true;
            self.goal_floor.fenced = true;
        }
        result
    }
}
