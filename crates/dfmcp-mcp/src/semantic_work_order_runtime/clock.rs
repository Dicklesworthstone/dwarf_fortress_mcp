//! Separately reviewed, finite native clock runs for one retained original order.
//!
//! The native callback owns stopping. Its receipts are historical clock evidence,
//! never original-goal proof, produced goods, or a current pause observation.
use super::{Work, denied, digest, exhausted, invalid};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::order_run::journal::{
    OrderRunEntry, OrderRunJournal, OrderRunMode, OrderRunState, OrderRunView,
};
use dfmcp_adapter::order_run::rpc::{OrderRunManifest, OrderRunSource, authorize_plan};
use dfmcp_adapter::order_run::{
    FortressIdentity, OrderCapture, OrderPredicate, OrderRunPlan, OrderRunRecord, OrderRunSpec,
};
use dfmcp_adapter::semantic_work_orders::{NativeOrderCreation, SemanticWorkOrderReview};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier,
};
use dfmcp_intent::PreparedPlan;
use dfmcp_intent::plan_archive::PlanArchiveSeal;
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

pub const MAX_RUNS: usize = 32;
const VIEW_BYTES: u64 = 4 * 1024 * 1024;
const ACTION_BYTES: u64 = 32 * 1024 * 1024;

fn corrupt(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message)
}
fn stale(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::StaleAnchor, message)
}
fn hash(domain: &[u8], fields: &[&[u8]]) -> Digest32 {
    let mut bytes = domain.to_vec();
    bytes.push(0);
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field);
    }
    Digest32::of_bytes(&bytes)
}

/// Created only from the exact retained semantic review and its verified insertion.
/// No client may select an arbitrary native order for this clock owner.
#[derive(Clone, Debug)]
pub struct ClockTarget {
    digest: Digest32,
    original_seal: Digest32,
    fortress: FortressIdentity,
    epoch: u64,
    original_tick: u64,
    order: u32,
    recipe: u8,
    amount: u32,
    deadline: u64,
}
impl ClockTarget {
    pub fn new(
        plan: &PreparedPlan,
        archive: PlanArchiveSeal,
        creation_journal: Digest32,
        review: &SemanticWorkOrderReview,
        creation: &NativeOrderCreation,
    ) -> Result<Self> {
        if plan != review.original_plan()
            || !plan.digest_is_valid()
            || !plan.id_is_valid()
            || archive.plan_digest != plan.digest
            || archive.object_digest == Digest32::ZERO
            || creation_journal == Digest32::ZERO
            || plan.steps.len() != 1
        {
            return Err(corrupt(
                "clock target differs from the retained original plan and custody",
            ));
        }
        let NativeOrderCreation::Inserted {
            native_order_id,
            receipt,
        } = creation
        else {
            return Err(denied(
                "clock control requires the original verified Created receipt",
            ));
        };
        if *receipt == Digest32::ZERO || *native_order_id > i32::MAX as u32 {
            return Err(corrupt("original insertion identity is invalid"));
        }
        let native = review.native_plan();
        let before = native.observation();
        if *native_order_id != before.next_order_id() {
            return Err(corrupt(
                "inserted native order differs from its original allocation identity",
            ));
        }
        let fortress = FortressIdentity::new(before.world_folder(), before.site_id())?;
        if fortress.fortress_id() != plan.anchor.fortress_id {
            return Err(corrupt("original order belongs to another fortress"));
        }
        let deadline = plan.steps[0]
            .obligation
            .as_ref()
            .ok_or_else(|| {
                denied("finite clock control requires the original absolute goal deadline")
            })?
            .deadline_tick
            .get();
        if deadline <= plan.anchor.tick.get() {
            return Err(corrupt(
                "original order deadline is not after its sealing observation",
            ));
        }
        let digest = hash(
            b"dfmcp-semantic-order-clock-target/1",
            &[
                plan.digest.as_bytes(),
                archive.object_digest.as_bytes(),
                creation_journal.as_bytes(),
                review.seal().as_bytes(),
                receipt.as_bytes(),
                &native_order_id.to_be_bytes(),
            ],
        );
        Ok(Self {
            digest,
            original_seal: review.seal(),
            fortress,
            epoch: plan.anchor.cursor.epoch,
            original_tick: plan.anchor.tick.get(),
            order: *native_order_id,
            recipe: native.spec().recipe() as u8,
            amount: native.spec().amount(),
            deadline,
        })
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    pub fn original_paired_seal(&self) -> Digest32 {
        self.original_seal
    }
    pub fn native_order_id(&self) -> u32 {
        self.order
    }
    pub fn deadline_tick(&self) -> u64 {
        self.deadline
    }
    fn context(&self, c: &OperationContext) -> Result<()> {
        if c.anchor.fortress_id != self.fortress.fortress_id()
            || c.anchor.cursor.epoch != self.epoch
            || c.anchor.tick.get() < self.original_tick
        {
            return Err(stale(
                "clock control cannot cross the original fortress, epoch or clock floor",
            ));
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
    }
    fn capture(&self, capture: &OrderCapture) -> Result<()> {
        if capture.fortress() != &self.fortress
            || capture.order_id() != self.order
            || !capture.eligible()
            || capture.recipe() != self.recipe
            || capture.total() != self.amount as i32
            || (capture.remaining() == 0 && capture.status() != 1)
            || capture.tick() < self.original_tick
        {
            return Err(stale(
                "clock capture is not the original finite order or a validated inactive completion",
            ));
        }
        Ok(())
    }
    fn plan(&self, plan: &OrderRunPlan) -> Result<()> {
        self.capture(plan.before())?;
        let spec = plan.spec();
        if spec.predicate() != stop_predicate(plan.before())
            || spec.samples() != 1
            || spec.interval() != 1
            || plan
                .before()
                .tick()
                .checked_add(u64::from(spec.game_ticks()))
                .is_none_or(|end| end > self.deadline)
            || plan.key() != self.key(plan.digest())
        {
            return Err(corrupt(
                "clock history changes its original target, stop predicate or deadline",
            ));
        }
        Ok(())
    }
    fn key(&self, plan: Digest32) -> String {
        let digest = hash(
            b"dfmcp-semantic-order-clock-key/1",
            &[self.digest.as_bytes(), plan.as_bytes()],
        );
        format!("semantic-clock-{}", digest.to_hex())
    }
    fn seal(&self, journal: Digest32, plan: &OrderRunPlan) -> Digest32 {
        hash(
            b"dfmcp-semantic-order-clock-review/1",
            &[
                self.digest.as_bytes(),
                journal.as_bytes(),
                &plan.canonical_bytes(),
            ],
        )
    }
}

fn stop_predicate(capture: &OrderCapture) -> OrderPredicate {
    if capture.remaining() > 0 {
        OrderPredicate::RemainingAtMost(0)
    } else {
        // A completed validated inactive order may still need advancing samples
        // for the original stability rule or independent physical quiescence.
        // Unexpected reactivation is an early-stop guard, never goal success.
        OrderPredicate::Active
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub kind: String,
    pub original_paired_seal: String,
    pub game_ticks: u32,
    pub wall_millis: u32,
}
impl RunRequest {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 2048 {
            return Err(exhausted("clock request exceeds 2048 bytes"));
        }
        let request: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed original-order clock request"))?;
        request.validate()?;
        Ok(request)
    }
    fn validate(&self) -> Result<()> {
        if self.kind != "run" {
            return Err(invalid("clock request kind must equal run"));
        }
        digest(&self.original_paired_seal)?;
        self.spec()?;
        Ok(())
    }
    fn spec(&self) -> Result<OrderRunSpec> {
        OrderRunSpec::new(
            self.game_ticks,
            self.wall_millis,
            OrderPredicate::RemainingAtMost(0),
            1,
            1,
        )
    }
}

#[derive(Clone, Debug)]
pub struct ClockRunView {
    pub review_seal: Digest32,
    pub entry: OrderRunEntry,
}
impl ClockRunView {
    pub fn to_json(&self) -> Value {
        let p = self.entry.plan();
        let native = self.entry.native().map(|n| json!({
            "phase":n.phase().as_str(), "reason":n.reason().as_str(), "trigger":n.trigger().as_str(),
            "unpause_attempted":n.unpause_attempted(), "historical_pause_verified":n.pause_verified(),
            "observed_tick":n.observed_tick(), "observed_tick_overshoot":n.observed_tick_overshoot(),
            "predicate_observed":n.predicate_observed(), "reported_stable_samples":n.reported_stable_samples(),
            "receipt":n.receipt().to_hex(), "current_pause_proven":false, "original_goal_proven":false,
        }));
        json!({"review_seal":self.review_seal.to_hex(), "key":p.key(), "plan_digest":p.digest().to_hex(),
            "state":self.entry.state().as_str(), "pending":!self.entry.settled(), "unresolved":self.entry.unresolved(),
            "native_order_id":p.before().order_id(), "before_tick":p.before().tick(),
            "source_generation":p.before().generation(), "source_sequence":p.before().sequence(),
            "game_ticks":p.spec().game_ticks(), "wall_millis":p.spec().wall_ms(),
            "stop_predicate":p.spec().predicate().as_str(), "threshold":0,
            "purpose":if p.before().remaining()>0 { "advance_original_order" } else { "advance_for_original_stability_or_quiescence" },
            "native":native})
    }
}
#[derive(Clone, Debug)]
pub struct ClockView {
    pub journal_id: Digest32,
    pub head: Digest32,
    pub mode: OrderRunMode,
    pub target_digest: Option<Digest32>,
    pub original_paired_seal: Option<Digest32>,
    pub runs: Vec<ClockRunView>,
    pub pending: bool,
    pub unresolved: bool,
    pub high_tick: u64,
    pub high_sequence: u64,
}
impl ClockView {
    pub fn has_runs(&self) -> bool {
        !self.runs.is_empty()
    }
    pub fn may_be_running(&self) -> bool {
        self.unresolved
    }
    /// Permission to attempt fresh read acquisition, never proof of present pause.
    pub fn can_sample_goal(&self) -> bool {
        !self.unresolved
    }
    pub fn active(&self) -> Option<&ClockRunView> {
        self.runs.iter().find(|r| !r.entry.settled())
    }
    /// Permanent source-loss evidence is local historical state. Querying it
    /// again cannot discover a pause in the replacement source.
    pub fn needs_reconciliation(&self) -> bool {
        self.active().is_some_and(|r| {
            !matches!(
                r.entry.state(),
                OrderRunState::Terminal | OrderRunState::CancelledBeforeDispatch
            )
        })
    }
    pub fn cancellation_needs_source(&self) -> bool {
        self.active().is_some_and(|r| {
            matches!(
                r.entry.state(),
                OrderRunState::DispatchStarted
                    | OrderRunState::Tracking
                    | OrderRunState::CancelRequested
            )
        })
    }
    pub fn to_json(&self) -> Value {
        json!({"journal_id":self.journal_id.to_hex(), "head":self.head.to_hex(),
            "mode":match self.mode { OrderRunMode::Control=>"control", OrderRunMode::Recover=>"reconcile", OrderRunMode::Offline=>"offline" },
            "target_digest":self.target_digest.map(|d|d.to_hex()),
            "original_paired_seal":self.original_paired_seal.map(|d|d.to_hex()),
            "run_count":self.runs.len(), "pending":self.pending, "unresolved":self.unresolved,
            "needs_reconciliation":self.needs_reconciliation(),
            "may_be_running":self.may_be_running(), "current_pause_proven":false,
            "original_goal_proven":false, "high_tick":self.high_tick,
            "high_source_sequence":self.high_sequence,
            "clock_floor_semantics":"native source floors only; rejected read floors are process-local",
            "active_run":self.active().map(ClockRunView::to_json),
            "runs":self.runs.iter().map(ClockRunView::to_json).collect::<Vec<_>>()})
    }
}

pub struct Clock<J: EffectJournalStorage> {
    journal: OrderRunJournal<J>,
    observed_tick_floor: u64,
    observed_sequence_floor: u64,
}
impl<J: EffectJournalStorage> Clock<J> {
    pub fn new(journal: OrderRunJournal<J>) -> Self {
        Self {
            journal,
            observed_tick_floor: 0,
            observed_sequence_floor: 0,
        }
    }
    pub fn mode(&self) -> OrderRunMode {
        self.journal.mode()
    }
    pub fn is_control(&self) -> bool {
        self.mode() == OrderRunMode::Control
    }
    fn view(&mut self, target: Option<&ClockTarget>, c: &OperationContext) -> Result<ClockView> {
        if let Some(t) = target {
            t.context(c)?;
        }
        let view = self.journal.view(c)?;
        let mut result = Self::validate(view, target)?;
        // Complete captures rejected by the cross-profile bracket still carry
        // source clock information. Keep those floors for this exclusive owner;
        // only native plans/receipts are durable in the existing journal codec.
        self.observed_tick_floor = self.observed_tick_floor.max(result.high_tick);
        self.observed_sequence_floor = self.observed_sequence_floor.max(result.high_sequence);
        result.high_tick = self.observed_tick_floor;
        result.high_sequence = self.observed_sequence_floor;
        Self::authorize_floor(&result, c, false)?;
        result.mode = self.mode();
        Ok(result)
    }
    fn authorize_floor(view: &ClockView, c: &OperationContext, clock: bool) -> Result<()> {
        // This is an authority check only. A native clock receipt is never
        // installed as a canonical anchor with the old state's hash.
        let mut current = c.clone();
        current.anchor.tick = GameTick(current.anchor.tick.get().max(view.high_tick));
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if clock {
            current.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        Ok(())
    }
    fn validate(view: OrderRunView, target: Option<&ClockTarget>) -> Result<ClockView> {
        if view.entries.len() > MAX_RUNS || (target.is_none() && !view.entries.is_empty()) {
            return Err(corrupt(
                "clock custody has excess runs or no exact original order association",
            ));
        }
        if target.is_some_and(|t| view.binding.fortress() != &t.fortress) {
            return Err(corrupt(
                "clock journal fortress differs from original order",
            ));
        }
        let mut runs = Vec::with_capacity(view.entries.len());
        let mut high_tick = target.map_or(0, |t| t.original_tick);
        let mut high_sequence = 0;
        for entry in view.entries {
            let t = target.ok_or_else(|| corrupt("clock history has no original target"))?;
            if view.binding.fortress() != &t.fortress {
                return Err(corrupt(
                    "clock journal fortress differs from original order",
                ));
            }
            t.plan(entry.plan())?;
            high_tick = high_tick.max(entry.plan().before().tick());
            let mut sequence = entry.plan().before().sequence();
            if let Some(n) = entry.native() {
                if let Some(tick) = n.observed_tick() {
                    high_tick = high_tick.max(tick);
                }
                if n.unpause_attempted() {
                    sequence = sequence
                        .checked_add(1)
                        .ok_or_else(|| corrupt("clock sequence overflow"))?;
                }
            }
            high_sequence = high_sequence.max(sequence);
            runs.push(ClockRunView {
                review_seal: t.seal(view.id, entry.plan()),
                entry,
            });
        }
        let pending = runs.iter().any(|r| !r.entry.settled());
        let unresolved = runs.iter().any(|r| r.entry.unresolved());
        if runs.iter().filter(|r| !r.entry.settled()).count() > 1 {
            return Err(corrupt("more than one unsettled original-order clock run"));
        }
        Ok(ClockView {
            journal_id: view.id,
            head: view.head,
            mode: OrderRunMode::Offline,
            target_digest: target.map(|t| t.digest),
            original_paired_seal: target.map(|t| t.original_seal),
            runs,
            pending,
            unresolved,
            high_tick,
            high_sequence,
        })
    }
    pub fn inspect(
        &mut self,
        target: Option<&ClockTarget>,
        c: &OperationContext,
    ) -> Result<ClockView> {
        let mut work = Work::new(c)?;
        let mut view = self.view(target, &work.reserve(VIEW_BYTES)?)?;
        view.mode = self.mode();
        Ok(view)
    }
    fn control(&self, target: &ClockTarget, c: &OperationContext) -> Result<()> {
        target.context(c)?;
        if !self.is_control() {
            return Err(denied("clock recovery mode cannot become control"));
        }
        c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
    }
    pub fn prepare<N: OrderRunSource>(
        &mut self,
        target: &ClockTarget,
        request: &RunRequest,
        source: &mut N,
        c: &OperationContext,
    ) -> Result<ClockView> {
        self.prepare_guarded(target, request, source, c, |_| Ok(()))
    }
    pub fn prepare_guarded<N, F>(
        &mut self,
        target: &ClockTarget,
        request: &RunRequest,
        source: &mut N,
        c: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: OrderRunSource,
        F: FnMut(&OperationContext) -> Result<()>,
    {
        self.control(target, c)?;
        request.validate()?;
        if digest(&request.original_paired_seal)? != target.original_seal {
            return Err(denied(
                "run request must name the exact original paired review",
            ));
        }
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        let before_view = self.view(Some(target), &work.reserve(VIEW_BYTES)?)?;
        // Refuse unresolved history before opening an observation path. Exact
        // duplicate prepared requests can be returned without preparing again.
        if let Some(active) = before_view.active() {
            let p = active.entry.plan();
            if p.spec().game_ticks() == request.game_ticks
                && p.spec().wall_ms() == request.wall_millis
                && p.before().tick() == c.anchor.tick.get()
            {
                return self.inspect(Some(target), &work.narrow(&output)?);
            }
            return Err(DfmcpError::new(
                ErrorCode::EffectIndeterminate,
                "settle or cancel the retained clock run before planning another",
            ));
        }
        let capture = self
            .journal
            .observe(source, target.order, &work.reserve(ACTION_BYTES)?)?;
        self.observed_tick_floor = self.observed_tick_floor.max(capture.tick());
        self.observed_sequence_floor = self.observed_sequence_floor.max(capture.sequence());
        target.capture(&capture)?;
        if capture.tick() != c.anchor.tick.get()
            || capture.tick() < before_view.high_tick
            || capture.sequence() < before_view.high_sequence
        {
            return Err(stale(
                "clock capture must match the fresh original-goal tick and retained clock floors",
            ));
        }
        let spec = OrderRunSpec::new(
            request.game_ticks,
            request.wall_millis,
            stop_predicate(&capture),
            1,
            1,
        )?;
        let provisional = OrderRunPlan::new("semantic-clock-candidate", spec, capture)?;
        let plan = OrderRunPlan::new(
            &target.key(provisional.digest()),
            provisional.spec(),
            provisional.before().clone(),
        )?;
        target.plan(&plan)?;
        authorize_plan(c, &plan)?;
        if before_view.runs.iter().any(|r| r.entry.plan() == &plan) {
            return self.inspect(Some(target), &work.narrow(&output)?);
        }
        if before_view.runs.len() == MAX_RUNS {
            return Err(exhausted("original order retains at most 32 clock runs"));
        }
        let mut checked = GuardedSource {
            inner: source,
            guard: &mut guard,
            edge: GuardEdge::Prepare,
        };
        self.journal
            .prepare(&mut checked, &plan, &work.reserve(ACTION_BYTES)?)?;
        self.inspect(Some(target), &work.narrow(&output)?)
    }
    pub fn commit<N: OrderRunSource>(
        &mut self,
        target: &ClockTarget,
        seal: Digest32,
        confirmed: bool,
        source: &mut N,
        c: &OperationContext,
    ) -> Result<ClockView> {
        self.commit_guarded(target, seal, confirmed, source, c, |_| Ok(()))
    }
    pub fn commit_guarded<N, F>(
        &mut self,
        target: &ClockTarget,
        seal: Digest32,
        confirmed: bool,
        source: &mut N,
        c: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: OrderRunSource,
        F: FnMut(&OperationContext) -> Result<()>,
    {
        self.control(target, c)?;
        if !confirmed {
            return Err(denied(
                "confirm the separately reviewed original-order clock run",
            ));
        }
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        let view = self.view(Some(target), &work.reserve(VIEW_BYTES)?)?;
        let run = view
            .runs
            .iter()
            .find(|r| r.review_seal == seal)
            .ok_or_else(|| {
                denied("clock commit requires the exact target-bound clock review seal")
            })?;
        if run.entry.settled() {
            return self.inspect(Some(target), &work.narrow(&output)?);
        }
        if run.entry.state() != OrderRunState::Prepared {
            return Err(DfmcpError::new(
                ErrorCode::EffectIndeterminate,
                "clock run is not dispatchable; reconcile the original run",
            ));
        }
        let p = run.entry.plan();
        if p.before().tick() != c.anchor.tick.get()
            || p.before().tick() < view.high_tick
            || p.before().sequence() < view.high_sequence
        {
            return Err(stale(
                "clock commit requires the fresh unchanged original-goal tick",
            ));
        }
        authorize_plan(c, p)?;
        let mut checked = GuardedSource {
            inner: source,
            guard: &mut guard,
            edge: GuardEdge::Commit,
        };
        self.journal.commit(
            &mut checked,
            p.key(),
            p.digest(),
            &work.reserve(ACTION_BYTES)?,
        )?;
        self.inspect(Some(target), &work.narrow(&output)?)
    }
    pub fn reconcile<N: OrderRunSource>(
        &mut self,
        target: &ClockTarget,
        source: &mut N,
        c: &OperationContext,
    ) -> Result<ClockView> {
        target.context(c)?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        let view = self.view(Some(target), &work.reserve(VIEW_BYTES)?)?;
        if let Some(run) = view.active() {
            self.journal.reconcile(
                source,
                run.entry.plan().key(),
                run.entry.plan().digest(),
                &work.reserve(ACTION_BYTES)?,
            )?;
        }
        self.inspect(Some(target), &work.narrow(&output)?)
    }
    pub fn cancel<N: OrderRunSource>(
        &mut self,
        target: &ClockTarget,
        source: Option<&mut N>,
        c: &OperationContext,
    ) -> Result<ClockView> {
        // Safety cancellation deliberately does not require the original plan
        // or goal still to be inside its deadline, nor a fresh Observe grant.
        target.context(c)?;
        if !self.is_control() {
            return Err(denied("clock recovery mode cannot request cancellation"));
        }
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        let view = self.view(Some(target), &work.reserve(VIEW_BYTES)?)?;
        Self::authorize_floor(&view, c, true)?;
        if let Some(run) = view.active() {
            self.journal.cancel(
                source,
                run.entry.plan().key(),
                run.entry.plan().digest(),
                &work.reserve(ACTION_BYTES)?,
            )?;
        }
        self.inspect(Some(target), &work.narrow(&output)?)
    }
}

enum GuardEdge {
    Prepare,
    Commit,
}
struct GuardedSource<'a, N, F> {
    inner: &'a mut N,
    guard: &'a mut F,
    edge: GuardEdge,
}
impl<N, F: FnMut(&OperationContext) -> Result<()>> GuardedSource<'_, N, F> {
    fn guarded_context(
        &mut self,
        c: &OperationContext,
        t: Duration,
    ) -> Result<(OperationContext, Duration)> {
        let started = Instant::now();
        (self.guard)(c)?;
        let timeout = t
            .checked_sub(started.elapsed())
            .filter(|t| *t >= Duration::from_millis(1))
            .ok_or_else(|| {
                exhausted("clock custody guard exhausted the native operation deadline")
            })?;
        let mut narrowed = c.clone();
        narrowed.budget.max_wall_millis = narrowed
            .budget
            .max_wall_millis
            .min(timeout.as_millis() as u64);
        if narrowed.budget.max_wall_millis == 0 {
            return Err(exhausted(
                "clock custody guard exhausted the native operation deadline",
            ));
        }
        Ok((narrowed, timeout))
    }
}
impl<N: OrderRunSource, F: FnMut(&OperationContext) -> Result<()>> OrderRunSource
    for GuardedSource<'_, N, F>
{
    fn manifest(&self) -> &OrderRunManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn fortress(&self) -> &FortressIdentity {
        self.inner.fortress()
    }
    fn fence(&mut self) {
        self.inner.fence();
    }
    fn observe(&mut self, id: u32, c: &OperationContext, t: Duration) -> Result<OrderCapture> {
        self.inner.observe(id, c, t)
    }
    fn prepare(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<OrderRunRecord> {
        if matches!(self.edge, GuardEdge::Prepare) {
            let (c, t) = self.guarded_context(c, t)?;
            return self.inner.prepare(p, &c, t);
        }
        self.inner.prepare(p, c, t)
    }
    fn commit(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<OrderRunRecord> {
        if matches!(self.edge, GuardEdge::Commit) {
            let (c, t) = self.guarded_context(c, t)?;
            return self.inner.commit(p, &c, t);
        }
        self.inner.commit(p, c, t)
    }
    fn query(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<Option<OrderRunRecord>> {
        self.inner.query(p, c, t)
    }
    fn cancel(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<OrderRunRecord> {
        self.inner.cancel(p, c, t)
    }
}

#[cfg(test)]
mod tests;
