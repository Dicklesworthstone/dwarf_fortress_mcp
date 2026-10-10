//! Durable original-goal observation custody and independently authorized replay.
//!
//! Raw captures and policy fingerprints do not import authority. Recovery binds
//! the exact original semantic/native review, reacquires historical source policy,
//! and replays the original predicates through the real obligation runtime.

use dfmcp_core::{Digest32, ErrorCode, GameTick, ObservationCursor, OperationContext, Result};
use dfmcp_world::EvidencePolicy;

use crate::control_effect_journal::EffectJournalStorage;
use crate::live_routing::LiveRoutingEvidence;
use crate::workforce_control::WorkforceCapture;
use crate::workforce_control::journal::WorkforceBinding;
use crate::workforce_control::rpc::WorkforceSource;

use super::goal_monitor::{
    GoalPublication, POLICY_RESERVE, PROJECTION_RESERVE, WorkforceGoalMonitor,
    WorkforceGoalProgress, WorkforceGoalResult, authorize_observation,
};
use super::projection::WorkforceGoalProjection;
use super::{Call, LOCAL_RESERVE, SemanticWorkforceSession, custody, error};

mod store;
use store::{Event, MAX_EVENT_BYTES, policy_fingerprint};
pub use store::{
    GoalHistoryStore, MAX_GOAL_HISTORY_BYTES, MAX_GOAL_HISTORY_EVENTS, PrivateGoalHistoryStore,
    open_private_goal_history_store,
};

/// One exact monitor and its exclusively owned history. Neither can be cloned,
/// imported as a caller assertion, or separated to bypass the retained history.
pub struct DurableWorkforceGoalMonitor<H> {
    monitor: WorkforceGoalMonitor,
    history: GoalHistoryStore<H>,
}
impl<H: EffectJournalStorage> DurableWorkforceGoalMonitor<H> {
    pub fn review_seal(&self) -> Digest32 {
        self.monitor.review_seal()
    }
    pub fn original_plan(&self) -> &dfmcp_intent::PreparedPlan {
        self.monitor.original_plan()
    }
    /// Always None immediately after restart. Replayed history is never fresh.
    pub fn latest(&self) -> Option<&WorkforceGoalProgress> {
        self.monitor.latest()
    }
    pub fn first_satisfied_anchor(&self) -> Option<dfmcp_core::StateAnchor> {
        self.monitor.first_satisfied_anchor()
    }
    pub fn last_observation_anchor(&self) -> Option<dfmcp_core::StateAnchor> {
        self.monitor.last_observation_anchor()
    }
    pub fn is_fenced(&self) -> bool {
        self.monitor.is_fenced() || self.history.is_fenced()
    }
    pub fn history_event_count(&self) -> usize {
        self.history.event_count()
    }
    pub fn history_byte_len(&self) -> usize {
        self.history.byte_len()
    }
    pub fn high_cursor(&self) -> ObservationCursor {
        self.history.high_cursor()
    }
    pub fn high_tick(&self) -> GameTick {
        self.history.high_tick()
    }
    /// Revalidated historical outcome; this is not proof of current game state.
    pub fn historical_obligation_status(&self) -> Option<&dfmcp_intent::ObligationStatus> {
        self.monitor
            .runtime
            .as_ref()
            .and_then(|r| r.get_status(self.monitor.action_id))
    }

    /// Persist an independently known interruption without acquiring a source or
    /// changing native work. Failed persistence fences the owner until reopening.
    pub fn observation_interrupted(&mut self, context: &OperationContext) -> Result<()> {
        self.monitor.observation_interrupted()?;
        let mut current = context.clone();
        current.anchor.tick = current
            .anchor
            .tick
            .max(self.history.high_tick())
            .max(self.monitor.high_tick);
        let result = self.history.append(
            Event::Interrupted {
                tick: current.anchor.tick,
                native: self
                    .history
                    .high_native_sequence()
                    .max(self.monitor.high_native_sequence),
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
        budget.edge(current)?;
        let event_bytes = match &event {
            Event::Captured(capture) => capture.canonical_bytes().len() + 1,
            Event::Started { .. } => 34,
            Event::Published(_) => 5,
            Event::Interrupted { .. } => 17,
        };
        let cost = LOCAL_RESERVE + 6 * (self.history.byte_len() + event_bytes + 128) as u64 + 8192;
        let mut context = budget.reserve(cost)?;
        // Floor custody is negative evidence. Query authorizes recording it;
        // only the monitor's independent Observe/source checks can publish proof.
        context.anchor.tick = context.anchor.tick.max(self.history.high_tick());
        self.history.append(event, &context)
    }
}
impl<H: EffectJournalStorage> GoalPublication for HistoryPublication<'_, H> {
    fn started(
        &mut self,
        monitor: &WorkforceGoalMonitor,
        cursor: ObservationCursor,
        call: &mut Call,
    ) -> Result<()> {
        self.history.preflight_poll(cursor)?;
        if self.history.event_count() + 4 > call.context()?.budget.max_entities as usize {
            return Err(super::exhausted());
        }
        // Prepay all append/verification work before source contact. Even failed
        // publication can retain its interruption and captured source clocks.
        let size = self.history.byte_len() + MAX_EVENT_BYTES + 512;
        let cost = LOCAL_RESERVE + 4 * (LOCAL_RESERVE + 6 * size as u64 + 8192);
        let mut budget = Call::new(&call.reserve(cost)?)?;
        let verification = budget.reserve(LOCAL_RESERVE + self.history.byte_len() as u64 + 8192)?;
        self.history.verify(&verification)?;
        self.budget = Some(budget);
        self.append(
            Event::Started {
                cursor,
                tick: monitor.high_tick.max(call.context()?.anchor.tick),
                native: monitor.high_native_sequence,
                interrupted: monitor.interrupted,
            },
            &call.context()?,
        )?;
        self.began = true;
        Ok(())
    }
    fn captured(&mut self, capture: &WorkforceCapture, call: &mut Call) -> Result<()> {
        self.append(Event::Captured(capture.clone()), &call.context()?)
    }
    fn published(
        &mut self,
        projection: &WorkforceGoalProjection,
        policy: &EvidencePolicy,
        call: &mut Call,
    ) -> Result<()> {
        self.append(
            Event::Published(policy_fingerprint(projection, policy)?),
            &call.context()?,
        )
    }
    fn failed(&mut self, monitor: &WorkforceGoalMonitor, context: &OperationContext) -> Result<()> {
        if !self.began {
            return Ok(());
        }
        self.append(
            Event::Interrupted {
                tick: monitor.high_tick.max(self.history.high_tick()),
                native: monitor
                    .high_native_sequence
                    .max(self.history.high_native_sequence()),
            },
            context,
        )
    }
}

impl<S: EffectJournalStorage, B: EffectJournalStorage> SemanticWorkforceSession<S, B> {
    /// Create or recover one durable foreground monitor. `history` must have been
    /// opened by the operator for this exact review and native journal. Recovery
    /// contacts no native source and never replays an assignment.
    ///
    /// Each retained Published capture must receive the same independently issued
    /// source policy twice. The callback receives the *current* authority floor in
    /// its context; the historical anchor is available from the projection. A
    /// denied or changed historical policy refuses recovery instead of restoring
    /// achievement from disk. No previously stored grant is reconstructed.
    pub fn begin_durable_goal_monitor<H, A>(
        &mut self,
        mut history: GoalHistoryStore<H>,
        seal: Digest32,
        original_identity: &LiveRoutingEvidence<'_>,
        context: &OperationContext,
        mut authorize_history: A,
    ) -> Result<DurableWorkforceGoalMonitor<H>>
    where
        H: EffectJournalStorage,
        A: FnMut(&WorkforceGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        let mut call = Call::new(context)?;
        call.context.anchor.tick = context.anchor.tick.max(history.high_tick());
        let verify = call.reserve(LOCAL_RESERVE + history.byte_len() as u64 + 8192)?;
        history.verify(&verify)?;
        let view = self.view(&mut call)?;
        let review = self.attached(seal)?;
        self.exact_association(&review)?;
        history.binding(view.id, &review)?;
        if !view
            .records
            .iter()
            .any(|record| record.plan() == &review.native)
        {
            return Err(custody());
        }
        call.reserve(PROJECTION_RESERVE)?;
        let mut monitor = WorkforceGoalMonitor::new(review.clone(), original_identity)?;
        authorize_observation(
            &monitor.identities,
            review.original.anchor,
            &call.context()?,
        )?;

        let mut pending_cursor = None;
        let mut pending_capture = None;
        for event in history.events() {
            call.context()?;
            match event {
                Event::Started {
                    cursor,
                    tick,
                    native,
                    interrupted,
                } => {
                    if pending_cursor.is_some() || *interrupted {
                        monitor.observation_interrupted()?;
                    }
                    pending_cursor = Some(*cursor);
                    pending_capture = None;
                    monitor.high_tick = monitor.high_tick.max(*tick);
                    monitor.high_native_sequence = monitor.high_native_sequence.max(*native);
                }
                Event::Captured(capture) => {
                    // Every captured source, even a denied publication, retains
                    // the original identity and nonregressing native fences.
                    monitor.identities.validate_capture(capture)?;
                    monitor.high_tick = monitor.high_tick.max(GameTick(capture.tick()));
                    monitor.high_native_sequence =
                        monitor.high_native_sequence.max(capture.sequence());
                    pending_capture = Some(capture);
                }
                Event::Published(fingerprint) => {
                    let cursor = pending_cursor.ok_or_else(custody)?;
                    let capture = pending_capture.take().ok_or_else(custody)?;
                    call.reserve(PROJECTION_RESERVE)?;
                    let policy_budget = call.reserve(POLICY_RESERVE)?;
                    let final_policy_budget = call.reserve(POLICY_RESERVE)?;
                    let projection = monitor.identities.project(capture.clone(), cursor)?;
                    let current = call.edge(&policy_budget)?;
                    authorize_observation(&monitor.identities, review.original.anchor, &current)?;
                    let policy = authorize_history(&projection, &current)?;
                    if policy_fingerprint(&projection, &policy)? != *fingerprint {
                        return Err(error(
                            ErrorCode::CapabilityDenied,
                            "historical workforce policy is unavailable or changed; stored history remains unverified",
                        ));
                    }
                    let evidence = projection.evidence(policy.clone())?;
                    let mut candidate = monitor.pending_sample();
                    candidate.sample(&projection, &evidence)?;
                    let current = call.edge(&final_policy_budget)?;
                    authorize_observation(&monitor.identities, review.original.anchor, &current)?;
                    if authorize_history(&projection, &current)? != policy {
                        return Err(error(
                            ErrorCode::CapabilityDenied,
                            "historical workforce policy changed before recovery publication",
                        ));
                    }
                    monitor = candidate;
                    pending_cursor = None;
                }
                Event::Interrupted { tick, native } => {
                    monitor.observation_interrupted()?;
                    monitor.high_tick = monitor.high_tick.max(*tick);
                    monitor.high_native_sequence = monitor.high_native_sequence.max(*native);
                    pending_cursor = None;
                    pending_capture = None;
                }
            }
        }
        // Restart is itself a discontinuity. Preserve verified terminal history
        // and cadence, but never count a stored capture as a new live sample.
        monitor.observation_interrupted()?;
        monitor.cursor_floor = history.high_cursor();
        if context.anchor.cursor.sequence > monitor.cursor_floor.sequence {
            monitor.cursor_floor = context.anchor.cursor;
        }
        monitor.high_tick = monitor
            .high_tick
            .max(call.context()?.anchor.tick)
            .max(history.high_tick());
        monitor.high_native_sequence = monitor
            .high_native_sequence
            .max(history.high_native_sequence());
        let verify = call.reserve(LOCAL_RESERVE + history.byte_len() as u64 + 8192)?;
        history.verify(&verify)?;
        let view = self.view(&mut call)?;
        self.exact_association(&review)?;
        history.binding(view.id, &review)?;
        authorize_observation(
            &monitor.identities,
            review.original.anchor,
            &call.context()?,
        )?;
        Ok(DurableWorkforceGoalMonitor { monitor, history })
    }

    /// One explicit fresh sample. Read intent is synced before native contact;
    /// raw capture and source floors before policy; the accepted policy marker
    /// before returning a current result. Failed reads never retry or mutate.
    pub fn poll_original_goal_durable<H, N, F, A>(
        &mut self,
        monitor: &mut DurableWorkforceGoalMonitor<H>,
        cursor: ObservationCursor,
        context: &OperationContext,
        factory: F,
        authorize: A,
    ) -> Result<WorkforceGoalResult>
    where
        H: EffectJournalStorage,
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
        A: FnMut(&WorkforceGoalProjection, &OperationContext) -> Result<EvidencePolicy>,
    {
        let mut publication = HistoryPublication {
            history: &mut monitor.history,
            budget: None,
            began: false,
        };
        self.poll_original_goal_with_publication(
            &mut monitor.monitor,
            cursor,
            context,
            factory,
            authorize,
            &mut publication,
        )
    }
}
