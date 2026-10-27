//! The actual Runtime composes original insertion, native clock coordination,
//! current goal proof and recovery. Only native responses/storage are injected.
use super::*;
use dfmcp_adapter::bounded_run::RunPhase;
use dfmcp_adapter::order_run::journal::{OrderRunBinding, OrderRunMode, OrderRunState};
use dfmcp_adapter::order_run::rpc::{OrderRunManifest, OrderRunSource, authorize, authorize_plan};
use dfmcp_adapter::order_run::{
    FortressIdentity, OrderCapture, OrderPredicate, OrderRunPlan, OrderRunRecord,
};
use std::net::SocketAddr;

#[derive(Clone)]
struct ClockMemory {
    inner: Memory,
    archive: Memory,
    corrupt_at: Rc<Cell<Option<usize>>>,
}
impl Read for ClockMemory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.inner.read(out)
    }
}
impl Write for ClockMemory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
impl Seek for ClockMemory {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}
impl EffectJournalStorage for ClockMemory {
    fn sync(&mut self) -> io::Result<()> {
        self.inner.sync()?;
        if self.corrupt_at.get() == Some(self.inner.data.borrow().syncs) {
            self.corrupt_at.set(None);
            let mut archive = self.archive.data.borrow_mut();
            if let Some(first) = archive.bytes.first_mut() {
                *first ^= 1;
            }
        }
        Ok(())
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        self.inner.truncate(length)
    }
    fn validate_identity(&self) -> io::Result<()> {
        self.inner.validate_identity()
    }
}
#[derive(Default)]
struct ClockState {
    records: BTreeMap<String, OrderRunRecord>,
    sequence: u64,
    prepares: usize,
    commits: usize,
    queries: usize,
    cancels: usize,
    observes: usize,
    connections: usize,
    lose_commit: bool,
}
#[derive(Clone)]
struct ClockResources {
    base: Resources,
    storage: ClockMemory,
    state: Rc<RefCell<ClockState>>,
    revoked: Rc<Cell<bool>>,
    expected_journal: Option<Digest32>,
}
impl ClockResources {
    fn new() -> Result<Self> {
        let base = Resources::new()?;
        let trace = Rc::clone(&base.native.borrow().trace);
        Ok(Self {
            storage: ClockMemory {
                inner: Memory::new("clock-journal", trace),
                archive: base.archive.clone(),
                corrupt_at: Rc::new(Cell::new(None)),
            },
            base,
            state: Rc::new(RefCell::new(ClockState {
                sequence: 1,
                ..ClockState::default()
            })),
            revoked: Rc::new(Cell::new(false)),
            expected_journal: None,
        })
    }
    fn source(&self) -> Result<ClockNative> {
        Ok(ClockNative {
            resources: self.clone(),
            manifest: OrderRunManifest {
                generation: 701,
                df_version: self.base.pins.df_version.clone(),
                dfhack_version: self.base.pins.dfhack_version.clone(),
            },
            fortress: FortressIdentity::new(&self.base.pins.world_folder, self.base.pins.site_id)?,
            fenced: false,
        })
    }
    fn paused(&self, paused: bool) -> Result<()> {
        let mut bytes = self.base.native.borrow().queue.canonical_bytes().to_vec();
        bytes[40] = u8::from(paused);
        self.base.native.borrow_mut().queue = WorkOrderObservation::decode(&bytes)?;
        Ok(())
    }
    fn stop(&self, ticks: u64, remaining: i32) -> Result<()> {
        let plan = self
            .state
            .borrow()
            .records
            .values()
            .find(|r| matches!(r.phase(), RunPhase::Running | RunPhase::Stopping))
            .map(|r| r.plan().clone())
            .ok_or_else(rejected)?;
        self.base.advance(ticks, remaining)?;
        self.paused(true)?;
        let tick = self.base.native.borrow().queue.tick();
        let receipt = clock_receipt(&plan, 3, 1, tick)?;
        self.state
            .borrow_mut()
            .records
            .insert(plan.key().into(), receipt);
        Ok(())
    }
    fn recover(&self, view: &View) -> Self {
        let mut result = self.clone();
        result.base = self.base.recover(view);
        result.expected_journal = view.clock.as_ref().map(|clock| clock.journal_id);
        result
    }
    fn calls(&self) -> (usize, usize, usize, usize, usize, usize, usize, usize) {
        let clock = self.state.borrow();
        let native = self.base.native.borrow();
        (
            clock.connections,
            clock.observes,
            clock.prepares,
            clock.commits,
            clock.queries,
            clock.cancels,
            native.reads,
            native.queries,
        )
    }
}
fn clock_capture(resources: &ClockResources) -> Result<OrderCapture> {
    let native = resources.base.native.borrow();
    let queue = &native.queue;
    let mut bytes = b"DFMOR014".to_vec();
    for value in [701u64, resources.state.borrow().sequence, queue.tick()] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&queue.site_id().to_be_bytes());
    text(&mut bytes, queue.world_folder());
    bytes.push(u8::from(queue.paused()));
    for value in [queue.next_order_id() - 1, queue.next_order_id()] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&[1, 1]);
    bytes.extend_from_slice(&5i32.to_be_bytes());
    let remaining = resources.base.remaining.get();
    bytes.extend_from_slice(&remaining.to_be_bytes());
    bytes.extend_from_slice(&(if remaining == 0 { 1u32 } else { 3u32 }).to_be_bytes());
    OrderCapture::decode(&bytes)
}
fn clock_receipt(plan: &OrderRunPlan, phase: u8, reason: u8, tick: u64) -> Result<OrderRunRecord> {
    let mut bytes = b"DFMOE014".to_vec();
    text(&mut bytes, plan.key());
    bytes.extend_from_slice(&(plan.native_bytes().len() as u16).to_be_bytes());
    bytes.extend_from_slice(plan.native_bytes());
    bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.token());
    let attempted = matches!(phase, 1..=3 | 5);
    let known = matches!(phase, 1..=3);
    bytes.extend_from_slice(&[
        phase,
        reason,
        0,
        u8::from(attempted),
        u8::from(phase == 3),
        u8::from(known),
    ]);
    bytes.extend_from_slice(&(if known { tick } else { 0 }).to_be_bytes());
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&plan.before().tick().to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes());
    let mut hash = b"dfmcp-order-run-receipt/1\0".to_vec();
    hash.extend_from_slice(&bytes);
    bytes.extend_from_slice(Digest32::of_bytes(&hash).as_bytes());
    OrderRunRecord::decode(&bytes)
}
struct ClockNative {
    resources: ClockResources,
    manifest: OrderRunManifest,
    fortress: FortressIdentity,
    fenced: bool,
}
impl ClockNative {
    fn check(&self, c: &OperationContext, control: bool) -> Result<()> {
        if self.fenced || (control && self.resources.revoked.get()) {
            return Err(rejected());
        }
        authorize(c, &self.fortress, control)
    }
}
impl OrderRunSource for ClockNative {
    fn manifest(&self) -> &OrderRunManifest {
        &self.manifest
    }
    fn fortress(&self) -> &FortressIdentity {
        &self.fortress
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(SocketAddr::from(([127, 0, 0, 1], 5000)))
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn observe(&mut self, id: u32, c: &OperationContext, _: Duration) -> Result<OrderCapture> {
        self.check(c, false)?;
        self.resources.state.borrow_mut().observes += 1;
        let value = clock_capture(&self.resources)?;
        if value.order_id() != id {
            return Err(rejected());
        }
        Ok(value)
    }
    fn prepare(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        _: Duration,
    ) -> Result<OrderRunRecord> {
        self.check(c, true)?;
        authorize_plan(c, p)?;
        let value = clock_receipt(p, 0, 0, 0)?;
        let mut state = self.resources.state.borrow_mut();
        state.prepares += 1;
        state.records.insert(p.key().into(), value.clone());
        Ok(value)
    }
    fn commit(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        _: Duration,
    ) -> Result<OrderRunRecord> {
        self.check(c, true)?;
        authorize_plan(c, p)?;
        let mut durable = OrderRunJournal::open(
            self.resources.storage.clone(),
            c,
            OrderRunMode::Offline,
            &self.fortress,
            None,
            false,
        )?;
        let persisted = durable.view(c)?;
        assert_eq!(
            persisted
                .entries
                .iter()
                .find(|e| e.plan() == p)
                .expect("durable dispatch")
                .state(),
            OrderRunState::DispatchStarted
        );
        self.resources.paused(false)?;
        let value = clock_receipt(p, 1, 0, p.before().tick())?;
        let mut state = self.resources.state.borrow_mut();
        state.commits += 1;
        state.sequence += 1;
        state.records.insert(p.key().into(), value.clone());
        if state.lose_commit {
            return Err(rejected());
        }
        Ok(value)
    }
    fn query(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        _: Duration,
    ) -> Result<Option<OrderRunRecord>> {
        self.check(c, false)?;
        let mut state = self.resources.state.borrow_mut();
        state.queries += 1;
        Ok(state.records.get(p.key()).cloned())
    }
    fn cancel(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        _: Duration,
    ) -> Result<OrderRunRecord> {
        self.check(c, true)?;
        self.resources.paused(true)?;
        let tick = self.resources.base.native.borrow().queue.tick();
        let value = clock_receipt(p, 3, 3, tick)?;
        let mut state = self.resources.state.borrow_mut();
        state.cancels += 1;
        state.records.insert(p.key().into(), value.clone());
        Ok(value)
    }
}
impl Shell for ClockResources {
    type Journal = Memory;
    type Association = Memory;
    type Native = Native;
    type Archive = Memory;
    type History = Memory;
    type Evidence = Evidence;
    type Progress = ProgressFixture;
    type Operations = OperationsFixture;
    type ClockJournal = ClockMemory;
    type ClockSource = ClockNative;
    fn acceptance(&self) -> &policy::SourceAcceptance {
        self.base.acceptance()
    }
    fn recovery(&self) -> Option<Recovery> {
        self.base.recovery()
    }
    fn check_environment(&self) -> Result<()> {
        self.base.check_environment()
    }
    fn production_enabled(&self) -> bool {
        self.base.production_enabled()
    }
    fn clock_enabled(&self) -> bool {
        !self.revoked.get()
    }
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        WorkOrderSession<Memory, Native>,
        AssociationStore<Memory>,
        Digest32,
    )> {
        let mode = if mode == Mode::Control && !self.production_enabled() {
            Mode::Reconcile
        } else {
            mode
        };
        self.base.open_control(mode, c)
    }
    fn acquire(&self, c: &OperationContext) -> Result<LiveObservationCapsule> {
        self.base.acquire(c)
    }
    fn evidence(&self, capsule: LiveObservationCapsule, c: &OperationContext) -> Result<Evidence> {
        self.base.evidence(capsule, c)
    }
    fn create_archive(
        &self,
        p: &PreparedPlan,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Memory>> {
        self.base.create_archive(p, c)
    }
    fn recover_archive(
        &self,
        r: Recovery,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Memory>> {
        self.base.recover_archive(r, c)
    }
    fn history(
        &self,
        j: Digest32,
        r: &SemanticWorkOrderReview,
        initialize: bool,
        read_only: bool,
        c: &OperationContext,
    ) -> Result<GoalHistoryStore<Memory>> {
        self.base.history(j, r, initialize, read_only, c)
    }
    fn progress(&self, c: &OperationContext) -> Result<ProgressFixture> {
        self.base.progress(c)
    }
    fn operations(&self, c: &OperationContext) -> Result<OperationsFixture> {
        self.base.operations(c)
    }
    fn open_clock(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<Option<OrderRunJournal<ClockMemory>>> {
        let mode = match mode {
            Mode::Offline => OrderRunMode::Offline,
            Mode::Reconcile => OrderRunMode::Recover,
            Mode::Control if self.clock_enabled() => OrderRunMode::Control,
            Mode::Control => OrderRunMode::Recover,
        };
        let source = self.source()?;
        let mut journal = OrderRunJournal::open(
            self.storage.clone(),
            c,
            mode,
            &source.fortress,
            Some(OrderRunBinding::from_source(&source)?),
            self.base.initialize,
        )?;
        if !self.base.initialize && self.expected_journal != Some(journal.view(c)?.id) {
            return Err(rejected());
        }
        Ok(Some(journal))
    }
    fn clock_source(&self, _: &OperationContext) -> Result<ClockNative> {
        self.state.borrow_mut().connections += 1;
        self.source()
    }
}
fn clock_budget() -> WorkBudget {
    WorkBudget {
        max_game_ticks: 1200,
        ..budget()
    }
}
fn start_clock(
    resources: &ClockResources,
    mode: Mode,
    session: u128,
) -> Result<Runtime<ClockResources>> {
    Runtime::from_shell(
        resources.clone(),
        mode,
        SessionId::new(session),
        clock_budget(),
    )
}
fn insert(runtime: &mut Runtime<ClockResources>) -> Result<View> {
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let request = Request::parse(
        &json!({
            "expected_observation":observed.observation.expect("observation").0.to_hex(),
            "recipe":"wooden_bed","amount":5,"deadline_tick":runtime.anchor().tick.get()+100,
            "poll_interval_ticks":10,"stable_samples":2
        })
        .to_string(),
    )?;
    let c = runtime.context(false)?;
    let prepared = runtime.plan(request, &c)?;
    let seal = prepared.review.as_ref().expect("paired review").seal();
    let c = runtime.context(false)?;
    runtime.commit(seal, true, &c)
}
fn run_request(view: &View, ticks: u32) -> RunRequest {
    RunRequest {
        kind: "run".into(),
        original_paired_seal: view.review.as_ref().expect("paired review").seal().to_hex(),
        game_ticks: ticks,
        wall_millis: 1000,
    }
}
fn planned_run(
    runtime: &mut Runtime<ClockResources>,
    view: &View,
    ticks: u32,
) -> Result<(View, Digest32)> {
    let c = runtime.context(false)?;
    let view = runtime.plan_run(run_request(view, ticks), &c)?;
    let seal = view
        .clock
        .as_ref()
        .and_then(ClockView::active)
        .expect("prepared run")
        .review_seal;
    Ok((view, seal))
}

#[test]
fn original_runtime_runs_to_completion_then_advances_original_sampled_stability() -> Result<()> {
    let resources = ClockResources::new()?;
    let mut runtime = start_clock(&resources, Mode::Control, 201)?;
    let inserted = insert(&mut runtime)?;
    let original = inserted.original.as_ref().expect("original").clone();
    let (_, seal) = planned_run(&mut runtime, &inserted, 10)?;
    let c = runtime.context(false)?;
    assert!(runtime.commit_run(seal, false, &c).is_err());
    let c = runtime.context(false)?;
    let running = runtime.commit_run(seal, true, &c)?;
    assert!(running.clock.as_ref().expect("clock").may_be_running());
    assert_eq!(running.anchor.tick, original.anchor.tick);
    assert!(running.observation.is_none());
    let progress_sequence = resources.base.sequence.get();
    let c = runtime.context(false)?;
    let pending = runtime.wait(&c)?;
    assert!(pending.clock.as_ref().expect("clock").may_be_running());
    assert_eq!(resources.base.sequence.get(), progress_sequence);
    assert_eq!(pending.anchor, running.anchor);
    resources.stop(10, 0)?;
    let c = runtime.context(false)?;
    let first = runtime.wait(&c)?;
    assert_eq!(
        first.goal.as_ref().expect("sample").current_goal,
        PredicateTruth::True
    );
    assert!(!first.goal.as_ref().expect("sample").current_goal_proven());
    let (next, seal) = planned_run(&mut runtime, &first, 10)?;
    let next_plan = next
        .clock
        .as_ref()
        .and_then(ClockView::active)
        .expect("next clock")
        .entry
        .plan();
    assert_eq!(next_plan.spec().predicate(), OrderPredicate::Active);
    assert_eq!(next_plan.before().remaining(), 0);
    let c = runtime.context(false)?;
    runtime.commit_run(seal, true, &c)?;
    resources.stop(10, 0)?;
    let c = runtime.context(false)?;
    let complete = runtime.wait(&c)?;
    let goal = complete.goal.as_ref().expect("sample");
    assert!(goal.current_goal_proven() && goal.physical_work.is_quiescent() && goal.receipt_linked);
    assert_eq!(complete.original.as_ref(), Some(&original));
    assert_eq!(resources.state.borrow().commits, 2);
    assert_eq!(resources.base.native.borrow().commits, 1);
    let c = runtime.context(false)?;
    assert!(runtime.plan_run(run_request(&complete, 10), &c).is_err());
    assert_eq!(resources.state.borrow().prepares, 2);
    Ok(())
}

#[test]
fn lost_clock_reply_and_offline_recovery_never_repeat_the_unpause() -> Result<()> {
    let resources = ClockResources::new()?;
    let mut runtime = start_clock(&resources, Mode::Control, 202)?;
    let inserted = insert(&mut runtime)?;
    let (_, seal) = planned_run(&mut runtime, &inserted, 10)?;
    resources.state.borrow_mut().lose_commit = true;
    let c = runtime.context(false)?;
    assert!(runtime.commit_run(seal, true, &c).is_err());
    resources.state.borrow_mut().lose_commit = false;
    let c = runtime.context(false)?;
    assert!(runtime.commit_run(seal, true, &c).is_err());
    assert_eq!(resources.state.borrow().commits, 1);
    let c = runtime.context(false)?;
    let retained = runtime.inspect(&c)?;
    assert!(retained.clock.as_ref().expect("clock").unresolved);
    let recovered = resources.recover(&retained);
    drop(runtime);
    let calls = resources.calls();
    let mut offline = start_clock(&recovered, Mode::Offline, 203)?;
    let c = offline.context(false)?;
    let historical = offline.wait(&c)?;
    assert!(historical.clock.as_ref().expect("clock").unresolved);
    assert!(historical.goal.is_none());
    assert_eq!(resources.calls(), calls);
    drop(offline);
    let mut read_only = start_clock(&recovered, Mode::Reconcile, 204)?;
    let c = read_only.context(false)?;
    assert!(read_only.commit_run(seal, true, &c).is_err());
    resources.stop(10, 0)?;
    let c = read_only.context(false)?;
    let observed = read_only.wait(&c)?;
    assert!(!observed.clock.as_ref().expect("clock").unresolved);
    assert_eq!(
        observed.goal.as_ref().expect("sample").current_goal,
        PredicateTruth::True
    );
    assert!(
        !observed
            .goal
            .as_ref()
            .expect("sample")
            .current_goal_proven()
    );
    assert_eq!(resources.state.borrow().commits, 1);
    assert_eq!(resources.base.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn clock_only_control_recovery_cannot_restore_insertion_or_promote_revoked_clock() -> Result<()> {
    let resources = ClockResources::new()?;
    let mut runtime = start_clock(&resources, Mode::Control, 205)?;
    let inserted = insert(&mut runtime)?;
    let (prepared, seal) = planned_run(&mut runtime, &inserted, 10)?;
    let recovered = resources.recover(&prepared);
    drop(runtime);
    resources.base.enabled.set(false);
    let mut clock_only = start_clock(&recovered, Mode::Control, 206)?;
    assert!(clock_only.clock_control_enabled());
    assert!(!clock_only.production_control_enabled());
    let c = clock_only.context(false)?;
    clock_only.commit_run(seal, true, &c)?;
    resources.base.enabled.set(true);
    assert!(!clock_only.production_control_enabled());
    let c = clock_only.context(false)?;
    assert!(
        clock_only
            .commit(inserted.review.as_ref().expect("review").seal(), true, &c)
            .is_err()
    );
    resources.base.enabled.set(false);
    let c = clock_only.context(false)?;
    let cancelled = clock_only.cancel_run(&c)?;
    assert_eq!(resources.state.borrow().cancels, 1);
    assert_eq!(resources.base.native.borrow().commits, 1);
    let recovered = resources.recover(&cancelled);
    drop(clock_only);
    resources.base.enabled.set(true);
    resources.revoked.set(true);
    let mut production_only = start_clock(&recovered, Mode::Control, 207)?;
    assert!(!production_only.clock_control_enabled());
    resources.revoked.set(false);
    assert!(!production_only.clock_control_enabled());
    let c = production_only.context(false)?;
    assert!(
        production_only
            .plan_run(run_request(&cancelled, 10), &c)
            .is_err()
    );
    Ok(())
}

#[test]
fn clock_run_cannot_change_original_seal_or_extend_original_deadline() -> Result<()> {
    let resources = ClockResources::new()?;
    let mut runtime = start_clock(&resources, Mode::Control, 208)?;
    let inserted = insert(&mut runtime)?;
    let original = inserted.original.as_ref().expect("original").clone();
    let mut request = run_request(&inserted, 10);
    request.original_paired_seal = Digest32::of_bytes(b"wrong-original-goal").to_hex();
    let c = runtime.context(false)?;
    assert!(runtime.plan_run(request, &c).is_err());
    let c = runtime.context(false)?;
    assert!(runtime.plan_run(run_request(&inserted, 101), &c).is_err());
    assert_eq!(resources.state.borrow().prepares, 0);
    assert_eq!(resources.state.borrow().commits, 0);
    let c = runtime.context(false)?;
    assert_eq!(runtime.inspect(&c)?.original, Some(original));
    Ok(())
}

#[test]
fn changed_original_archive_after_dispatch_sync_blocks_actual_clock_mutation() -> Result<()> {
    let resources = ClockResources::new()?;
    let mut runtime = start_clock(&resources, Mode::Control, 209)?;
    let inserted = insert(&mut runtime)?;
    let (_, seal) = planned_run(&mut runtime, &inserted, 10)?;
    let sync = resources.storage.inner.data.borrow().syncs + 1;
    resources.storage.corrupt_at.set(Some(sync));
    let c = runtime.context(false)?;
    assert!(runtime.commit_run(seal, true, &c).is_err());
    assert_eq!(resources.state.borrow().commits, 0);
    assert!(resources.base.native.borrow().queue.paused());
    let source = resources.source()?;
    let mut journal = OrderRunJournal::open(
        resources.storage.clone(),
        &c,
        OrderRunMode::Offline,
        &source.fortress,
        None,
        false,
    )?;
    assert_eq!(
        journal.view(&c)?.entries[0].state(),
        OrderRunState::DispatchStarted
    );
    let c = runtime.context(false)?;
    assert!(runtime.inspect(&c).is_err());
    Ok(())
}

#[test]
fn revoked_clock_after_dispatch_sync_retains_uncertainty_without_unpausing() -> Result<()> {
    let resources = ClockResources::new()?;
    let mut runtime = start_clock(&resources, Mode::Control, 210)?;
    let inserted = insert(&mut runtime)?;
    let (_, seal) = planned_run(&mut runtime, &inserted, 10)?;
    let sync = resources.storage.inner.data.borrow().syncs + 1;
    resources.storage.inner.data.borrow_mut().deny_sync =
        Some((sync, Rc::clone(&resources.revoked)));
    let c = runtime.context(false)?;
    assert!(runtime.commit_run(seal, true, &c).is_err());
    assert_eq!(resources.state.borrow().commits, 0);
    let c = runtime.context(false)?;
    let retained = runtime.inspect(&c)?;
    assert!(retained.clock.as_ref().expect("clock").unresolved);
    assert!(resources.base.native.borrow().queue.paused());
    Ok(())
}
