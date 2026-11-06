//! Real standalone runtime, both durable clock owners and original goal monitor.
//! Only native responses and storage are fixtures; run dispatch/replay is real.
use super::*;
use dfmcp_adapter::bounded_run::journal::{RunBinding, RunJournal, RunMode, RunState};
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunSource};
use dfmcp_adapter::bounded_run::{RunObservation, RunPlan, RunRecord};
use dfmcp_core::FortressId;

#[derive(Default)]
struct InventoryState {
    records: BTreeMap<String, RunRecord>,
    sequence: u64,
    prepares: usize,
    commits: usize,
    queries: usize,
    cancels: usize,
    connects: usize,
    lose_commit: bool,
}
#[derive(Clone)]
struct InventoryResources {
    order: ClockResources,
    storage: ClockMemory,
    state: Rc<RefCell<InventoryState>>,
    revoked: Rc<Cell<bool>>,
    expected: Option<Digest32>,
}
impl InventoryResources {
    fn new() -> Result<Self> {
        let order = ClockResources::new()?;
        let trace = Rc::clone(&order.base.native.borrow().trace);
        Ok(Self {
            storage: ClockMemory {
                inner: Memory::new("inventory-clock", trace),
                archive: order.base.archive.clone(),
                corrupt_at: Rc::new(Cell::new(None)),
            },
            order,
            state: Rc::new(RefCell::new(InventoryState {
                sequence: 1,
                ..InventoryState::default()
            })),
            revoked: Rc::new(Cell::new(false)),
            expected: None,
        })
    }
    fn source(&self) -> InventoryNative {
        InventoryNative {
            resources: self.clone(),
            manifest: RunManifest {
                generation: 801,
                df_version: self.order.base.pins.df_version.clone(),
                dfhack_version: self.order.base.pins.dfhack_version.clone(),
            },
        }
    }
    fn stop(&self, ticks: u64) -> Result<()> {
        let plan = self
            .state
            .borrow()
            .records
            .values()
            .find(|r| matches!(r.phase(), RunPhase::Running | RunPhase::Stopping))
            .map(|r| r.plan().clone())
            .ok_or_else(rejected)?;
        self.order.base.advance(ticks, 0)?;
        self.order.paused(true)?;
        let tick = self.order.base.native.borrow().queue.tick();
        self.state
            .borrow_mut()
            .records
            .insert(plan.key().into(), inventory_receipt(&plan, 3, 1, tick)?);
        Ok(())
    }
    fn recover(&self, view: &View) -> Self {
        let mut next = self.clone();
        next.order = self.order.recover(view);
        next.expected = view.inventory_clock.as_ref().map(|clock| clock.journal_id);
        next
    }
}
fn inventory_receipt(plan: &RunPlan, phase: u8, reason: u8, tick: u64) -> Result<RunRecord> {
    let mut out = b"DFMRE013".to_vec();
    text(&mut out, plan.key());
    out.extend_from_slice(&plan.spec().game_ticks().to_be_bytes());
    out.extend_from_slice(&plan.spec().wall_ms().to_be_bytes());
    out.extend_from_slice(plan.before().canonical_bytes());
    out.extend_from_slice(plan.digest().as_bytes());
    out.extend_from_slice(plan.token());
    let known = matches!(phase, 1..=3);
    out.extend_from_slice(&[
        phase,
        reason,
        u8::from(matches!(phase, 1..=3 | 5)),
        u8::from(phase == 3),
        u8::from(known),
    ]);
    out.extend_from_slice(&(if known { tick } else { 0 }).to_be_bytes());
    let mut hash = b"dfmcp-bounded-run-receipt/1\0".to_vec();
    hash.extend_from_slice(&out);
    out.extend_from_slice(Digest32::of_bytes(&hash).as_bytes());
    RunRecord::decode(&out)
}
struct InventoryNative {
    resources: InventoryResources,
    manifest: RunManifest,
}
impl InventoryNative {
    fn check(&self, c: &OperationContext, control: bool) -> Result<()> {
        assert_eq!(c.anchor.fortress_id, FortressId::NIL);
        assert!(
            c.grants
                .iter()
                .all(|grant| grant.scope.fortress_id == Some(FortressId::NIL))
        );
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if control {
            if self.resources.revoked.get() {
                return Err(rejected());
            }
            c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        Ok(())
    }
    fn durable(&self, c: &OperationContext, p: &RunPlan, state: RunState) -> Result<()> {
        let mut journal = RunJournal::open(
            self.resources.storage.clone(),
            c,
            RunMode::Offline,
            None,
            false,
        )?;
        assert_eq!(
            journal
                .records(c)?
                .iter()
                .find(|entry| entry.plan() == p)
                .expect("durable marker")
                .state(),
            state
        );
        Ok(())
    }
}
impl RunSource for InventoryNative {
    fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(SocketAddr::from(([127, 0, 0, 1], 5000)))
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        self.check(c, false)?;
        let state = self.resources.state.borrow();
        let native = self.resources.order.base.native.borrow();
        let queue = &native.queue;
        let mut out = b"DFMRO013".to_vec();
        for n in [801, state.sequence, queue.tick()] {
            out.extend_from_slice(&n.to_be_bytes());
        }
        out.extend_from_slice(&[1, 1, u8::from(queue.paused())]);
        RunObservation::decode(&out)
    }
    fn prepare(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        self.durable(c, p, RunState::Intent)?;
        let record = inventory_receipt(p, 0, 0, 0)?;
        let mut state = self.resources.state.borrow_mut();
        state.prepares += 1;
        state.records.insert(p.key().into(), record.clone());
        Ok(record)
    }
    fn commit(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        self.durable(c, p, RunState::DispatchStarted)?;
        self.resources.order.paused(false)?;
        let record = inventory_receipt(p, 1, 0, p.before().tick().expect("before tick"))?;
        let mut state = self.resources.state.borrow_mut();
        state.commits += 1;
        state.sequence += 1;
        state.records.insert(p.key().into(), record.clone());
        if state.lose_commit {
            return Err(rejected());
        }
        Ok(record)
    }
    fn query(&mut self, p: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        self.check(c, false)?;
        let mut state = self.resources.state.borrow_mut();
        state.queries += 1;
        Ok(state.records.get(p.key()).cloned())
    }
    fn cancel(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        self.durable(c, p, RunState::CancelRequested)?;
        self.resources.order.paused(true)?;
        let tick = self.resources.order.base.native.borrow().queue.tick();
        let record = inventory_receipt(p, 3, 3, tick)?;
        let mut state = self.resources.state.borrow_mut();
        state.cancels += 1;
        state.records.insert(p.key().into(), record.clone());
        Ok(record)
    }
}
impl Shell for InventoryResources {
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
    type InventoryClockJournal = ClockMemory;
    type InventoryClockSource = InventoryNative;
    fn acceptance(&self) -> &policy::SourceAcceptance {
        self.order.acceptance()
    }
    fn recovery(&self) -> Option<Recovery> {
        self.order.recovery()
    }
    fn check_environment(&self) -> Result<()> {
        self.order.check_environment()
    }
    fn production_enabled(&self) -> bool {
        self.order.production_enabled()
    }
    fn clock_enabled(&self) -> bool {
        self.order.clock_enabled()
    }
    fn open_control(
        &self,
        m: Mode,
        c: &OperationContext,
    ) -> Result<OpenControl<Memory, Native, Memory>> {
        self.order.open_control(m, c)
    }
    fn acquire(&self, c: &OperationContext) -> Result<LiveObservationCapsule> {
        self.order.acquire(c)
    }
    fn evidence(&self, b: LiveObservationCapsule, c: &OperationContext) -> Result<Evidence> {
        self.order.evidence(b, c)
    }
    fn create_archive(
        &self,
        p: &PreparedPlan,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Memory>> {
        self.order.create_archive(p, c)
    }
    fn recover_archive(
        &self,
        r: Recovery,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Memory>> {
        self.order.recover_archive(r, c)
    }
    fn history(
        &self,
        j: Digest32,
        r: &SemanticWorkOrderReview,
        i: bool,
        ro: bool,
        c: &OperationContext,
    ) -> Result<GoalHistoryStore<Memory>> {
        self.order.history(j, r, i, ro, c)
    }
    fn progress(&self, c: &OperationContext) -> Result<ProgressFixture> {
        self.order.progress(c)
    }
    fn operations(&self, c: &OperationContext) -> Result<OperationsFixture> {
        self.order.operations(c)
    }
    fn open_clock(
        &self,
        m: Mode,
        c: &OperationContext,
    ) -> Result<Option<OrderRunJournal<ClockMemory>>> {
        self.order.open_clock(m, c)
    }
    fn clock_source(&self, c: &OperationContext) -> Result<ClockNative> {
        self.order.clock_source(c)
    }
    fn inventory_clock_configured(&self) -> bool {
        true
    }
    fn inventory_clock_enabled(&self) -> bool {
        !self.revoked.get()
    }
    fn inventory_clock_context(
        &self,
        c: &OperationContext,
        control: bool,
    ) -> Result<OperationContext> {
        assert_eq!(c.anchor.fortress_id, self.acceptance().fortress());
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if control {
            if self.revoked.get() {
                return Err(rejected());
            }
            c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        let mut native = c.clone();
        native.anchor = StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor::ORIGIN,
            tick: c.anchor.tick,
            state_hash: Digest32::ZERO,
        };
        let capabilities = if control {
            vec![
                Capability::Query,
                Capability::Plan,
                Capability::ControlClock,
            ]
        } else {
            vec![Capability::Query]
        };
        native.grants = capabilities
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(FortressId::NIL),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect();
        Ok(native)
    }
    fn open_inventory_clock(
        &self,
        m: Mode,
        c: &OperationContext,
    ) -> Result<Option<RunJournal<ClockMemory>>> {
        let mode = match m {
            Mode::Offline => RunMode::Offline,
            Mode::Control if self.inventory_clock_enabled() => RunMode::Control,
            _ => RunMode::Recover,
        };
        let native = self.inventory_clock_context(c, mode == RunMode::Control)?;
        let source = self.source();
        let mut journal = RunJournal::open(
            self.storage.clone(),
            &native,
            mode,
            Some(RunBinding::new(
                source.endpoint().expect("endpoint"),
                source.manifest.clone(),
            )?),
            self.order.base.initialize,
        )?;
        journal.records(&native)?;
        if !self.order.base.initialize && self.expected != Some(journal.id()) {
            return Err(rejected());
        }
        Ok(Some(journal))
    }
    fn inventory_clock_source(&self, c: &OperationContext) -> Result<InventoryNative> {
        let source = self.source();
        source.check(c, false)?;
        self.state.borrow_mut().connects += 1;
        Ok(source)
    }
}
fn start_inventory(
    resources: &InventoryResources,
    mode: Mode,
    session: u128,
) -> Result<Runtime<InventoryResources>> {
    Runtime::from_shell(
        resources.clone(),
        mode,
        SessionId::new(session),
        clock_budget(),
    )
}
fn insert_inventory(
    runtime: &mut Runtime<InventoryResources>,
    samples: u32,
    explicit: bool,
) -> Result<View> {
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let mut request = json!({"expected_observation":observed.observation.expect("observation").0.to_hex(),"recipe":"wooden_bed","amount":5,
        "deadline_tick":runtime.anchor().tick.get()+100,"poll_interval_ticks":10,"stable_samples":samples});
    if explicit {
        request["objective"] =
            json!({"kind":"observed_inventory_minimum","item_type":"BED","minimum_stack_units":1});
    }
    let c = runtime.context(false)?;
    let prepared = runtime.plan(Request::parse(&request.to_string())?, &c)?;
    let c = runtime.context(false)?;
    runtime.commit(prepared.review.as_ref().expect("review").seal(), true, &c)
}
fn retire(resources: &InventoryResources) -> Result<()> {
    let queue = resources.order.base.native.borrow().queue.clone();
    let mut bytes = queue.canonical_bytes().to_vec();
    let offset = 43 + queue.world_folder().len();
    bytes[offset..offset + 4]
        .copy_from_slice(&((queue.order_ids().len() - 1) as u32).to_be_bytes());
    bytes.truncate(bytes.len() - 4);
    resources.order.base.native.borrow_mut().queue = WorkOrderObservation::decode(&bytes)?;
    Ok(())
}
fn inventory_request(view: &View, ticks: u32) -> Result<InventoryRunRequest> {
    InventoryRunRequest::parse(&json!({"kind":"inventory_run",
    "original_paired_seal":view.review.as_ref().expect("review").seal().to_hex(),"game_ticks":ticks,"wall_millis":1000}).to_string())
}
fn inventory_plan(
    runtime: &mut Runtime<InventoryResources>,
    view: &View,
    ticks: u32,
) -> Result<(View, Digest32)> {
    let c = runtime.context(false)?;
    let prepared = runtime.plan_inventory_run(inventory_request(view, ticks)?, &c)?;
    let seal = prepared
        .inventory_clock
        .as_ref()
        .and_then(InventoryClockView::active)
        .expect("prepared inventory run")
        .review_seal;
    Ok((prepared, seal))
}
fn true_inventory(
    resources: &InventoryResources,
    runtime: &mut Runtime<InventoryResources>,
    retired: bool,
) -> Result<View> {
    resources.order.base.item_count.set(1);
    resources.order.base.advance(10, 0)?;
    if retired {
        retire(resources)?;
    }
    let c = runtime.context(false)?;
    runtime.wait(&c)
}

#[test]
fn retired_inventory_advances_original_stability_without_order_reconstruction() -> Result<()> {
    let resources = InventoryResources::new()?;
    let mut runtime = start_inventory(&resources, Mode::Control, 301)?;
    let inserted = insert_inventory(&mut runtime, 2, true)?;
    let first = true_inventory(&resources, &mut runtime, true)?;
    assert!(!first.goal.as_ref().expect("goal").receipt_linked);
    assert!(!first.goal.as_ref().expect("goal").current_goal_proven());
    let packet = crate::live_semantic_work_orders_server::test_packet_for_view(
        &first,
        runtime.session_id(),
        clock_budget(),
    );
    let packet: Value = serde_json::from_str(&packet).expect("packet");
    let recommendation = &packet["agent_turn"]["recommendations"][0];
    let request: Value = serde_json::from_str(
        recommendation["arguments"]["request"]
            .as_str()
            .expect("inventory request"),
    )
    .expect("request");
    assert_eq!(request["kind"], "inventory_run");
    assert_eq!(request["game_ticks"], 10);
    let (_, seal) = inventory_plan(&mut runtime, &first, 10)?;
    let c = runtime.context(false)?;
    assert!(runtime.commit_inventory_run(seal, false, &c).is_err());
    let c = runtime.context(false)?;
    let running = runtime.commit_inventory_run(seal, true, &c)?;
    assert!(running.goal.is_none());
    assert!(running.inventory_clock.as_ref().expect("clock").unresolved);
    let progress = resources.order.base.sequence.get();
    let c = runtime.context(false)?;
    assert!(runtime.wait(&c)?.goal.is_none());
    assert_eq!(resources.order.base.sequence.get(), progress);
    resources.stop(10)?;
    let c = runtime.context(false)?;
    let complete = runtime.wait(&c)?;
    assert!(complete.goal.as_ref().expect("goal").current_goal_proven());
    assert!(!complete.goal.as_ref().expect("goal").receipt_linked);
    assert_eq!(complete.original, inserted.original);
    assert_eq!(resources.state.borrow().commits, 1);
    assert_eq!(resources.order.state.borrow().commits, 0);
    assert_eq!(resources.order.base.native.borrow().commits, 1);
    let c = runtime.context(false)?;
    assert!(
        runtime
            .plan_inventory_run(inventory_request(&complete, 10)?, &c)
            .is_err()
    );
    Ok(())
}

#[test]
fn inventory_clock_refuses_false_unknown_proved_legacy_and_horizon_changes() -> Result<()> {
    for (samples, explicit, items, unknown) in [
        (2, true, 0, false),
        (2, true, 1, true),
        (1, true, 1, false),
        (2, false, 1, false),
    ] {
        let resources = InventoryResources::new()?;
        let mut runtime = start_inventory(&resources, Mode::Control, 302)?;
        let inserted = insert_inventory(&mut runtime, samples, explicit)?;
        resources.order.base.item_count.set(items);
        resources.order.base.advance(10, 0)?;
        resources
            .order
            .base
            .changed_operations_generation
            .set(unknown);
        let c = runtime.context(false)?;
        assert!(
            runtime
                .plan_inventory_run(inventory_request(&inserted, 10)?, &c)
                .is_err()
        );
        assert_eq!(resources.state.borrow().commits, 0);
        assert_eq!(resources.state.borrow().prepares, 0);
        assert_eq!(resources.state.borrow().connects, 0);
    }
    let resources = InventoryResources::new()?;
    let mut runtime = start_inventory(&resources, Mode::Control, 303)?;
    insert_inventory(&mut runtime, 2, true)?;
    let first = true_inventory(&resources, &mut runtime, true)?;
    let c = runtime.context(false)?;
    assert!(
        runtime
            .plan_inventory_run(inventory_request(&first, 91)?, &c)
            .is_err()
    );
    assert_eq!(resources.state.borrow().prepares, 0);
    let mut c = runtime.context(false)?;
    c.budget.max_game_ticks = 9;
    assert!(
        runtime
            .plan_inventory_run(inventory_request(&first, 10)?, &c)
            .is_err()
    );
    assert_eq!(resources.state.borrow().prepares, 0);
    Ok(())
}

#[test]
fn prepared_clocks_mutually_fence_each_other_without_silent_switching() -> Result<()> {
    let resources = InventoryResources::new()?;
    let mut runtime = start_inventory(&resources, Mode::Control, 304)?;
    insert_inventory(&mut runtime, 3, true)?;
    let first = true_inventory(&resources, &mut runtime, false)?;
    let c = runtime.context(false)?;
    let order = runtime.plan_run(run_request(&first, 10), &c)?;
    let c = runtime.context(false)?;
    assert!(
        runtime
            .plan_inventory_run(inventory_request(&order, 10)?, &c)
            .is_err()
    );
    assert_eq!(resources.state.borrow().prepares, 0);
    let c = runtime.context(false)?;
    runtime.cancel_run(&c)?;
    let (inventory, _) = inventory_plan(&mut runtime, &first, 10)?;
    let c = runtime.context(false)?;
    assert!(runtime.plan_run(run_request(&inventory, 10), &c).is_err());
    assert_eq!(resources.order.state.borrow().prepares, 1);
    assert_eq!(resources.state.borrow().prepares, 1);
    assert_eq!(resources.order.state.borrow().commits, 0);
    assert_eq!(resources.state.borrow().commits, 0);
    Ok(())
}

#[test]
fn inventory_lost_reply_recovery_queries_once_and_never_repeats_unpause() -> Result<()> {
    let resources = InventoryResources::new()?;
    let mut runtime = start_inventory(&resources, Mode::Control, 305)?;
    insert_inventory(&mut runtime, 2, true)?;
    let first = true_inventory(&resources, &mut runtime, true)?;
    let (_, seal) = inventory_plan(&mut runtime, &first, 10)?;
    resources.state.borrow_mut().lose_commit = true;
    let c = runtime.context(false)?;
    assert!(runtime.commit_inventory_run(seal, true, &c).is_err());
    resources.state.borrow_mut().lose_commit = false;
    let c = runtime.context(false)?;
    assert!(runtime.commit_inventory_run(seal, true, &c).is_err());
    let c = runtime.context(false)?;
    let retained = runtime.inspect(&c)?;
    assert!(retained.inventory_clock.as_ref().expect("clock").unresolved);
    let handoff = resources.recover(&retained);
    drop(runtime);
    let connects = resources.state.borrow().connects;
    let mut offline = start_inventory(&handoff, Mode::Offline, 306)?;
    let c = offline.context(false)?;
    assert!(offline.wait(&c)?.goal.is_none());
    assert_eq!(resources.state.borrow().connects, connects);
    drop(offline);
    let mut recovered = start_inventory(&handoff, Mode::Reconcile, 307)?;
    let c = recovered.context(false)?;
    assert!(recovered.commit_inventory_run(seal, true, &c).is_err());
    resources.stop(10)?;
    let c = recovered.context(false)?;
    let settled = recovered.wait(&c)?;
    assert!(!settled.inventory_clock.as_ref().expect("clock").unresolved);
    assert_eq!(resources.state.borrow().commits, 1);
    assert_eq!(resources.order.base.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn inventory_post_marker_policy_or_archive_change_refuses_native_effect() -> Result<()> {
    for archive in [false, true] {
        let resources = InventoryResources::new()?;
        let mut runtime = start_inventory(&resources, Mode::Control, 308)?;
        insert_inventory(&mut runtime, 2, true)?;
        let first = true_inventory(&resources, &mut runtime, true)?;
        let (_, seal) = inventory_plan(&mut runtime, &first, 10)?;
        let sync = resources.storage.inner.data.borrow().syncs + 1;
        if archive {
            resources.storage.corrupt_at.set(Some(sync));
        } else {
            resources.storage.inner.data.borrow_mut().deny_sync = Some((
                sync,
                Rc::clone(&resources.order.base.changed_operations_generation),
            ));
        }
        let c = runtime.context(false)?;
        assert!(runtime.commit_inventory_run(seal, true, &c).is_err());
        assert_eq!(resources.state.borrow().commits, 0);
        assert!(resources.order.base.native.borrow().queue.paused());
        let native = resources.inventory_clock_context(&c, false)?;
        let mut journal = RunJournal::open(
            resources.storage.clone(),
            &native,
            RunMode::Offline,
            None,
            false,
        )?;
        assert_eq!(
            journal.records(&native)?[0].state(),
            RunState::DispatchStarted
        );
    }
    Ok(())
}

#[test]
fn inventory_cancel_after_deadline_needs_clock_query_only_and_opening_cannot_promote() -> Result<()>
{
    let resources = InventoryResources::new()?;
    let mut runtime = start_inventory(&resources, Mode::Control, 309)?;
    insert_inventory(&mut runtime, 2, true)?;
    let first = true_inventory(&resources, &mut runtime, true)?;
    let (_, seal) = inventory_plan(&mut runtime, &first, 10)?;
    let c = runtime.context(false)?;
    runtime.commit_inventory_run(seal, true, &c)?;
    resources.order.base.advance(110, 0)?;
    let mut c = runtime.context(false)?;
    c.grants.retain(|grant| {
        matches!(
            grant.capability,
            Capability::Query | Capability::ControlClock
        )
    });
    let cancelled = runtime.cancel_inventory_run(&c)?;
    assert_eq!(resources.state.borrow().cancels, 1);
    assert!(cancelled.goal.is_none());
    let handoff = resources.recover(&cancelled);
    drop(runtime);
    resources.revoked.set(true);
    let mut recovered = start_inventory(&handoff, Mode::Control, 310)?;
    resources.revoked.set(false);
    assert!(!recovered.inventory_clock_control_enabled());
    let c = recovered.context(false)?;
    assert!(
        recovered
            .plan_inventory_run(inventory_request(&cancelled, 10)?, &c)
            .is_err()
    );
    Ok(())
}

#[test]
fn inventory_handoff_requires_both_exact_journal_identities() -> Result<()> {
    let resources = InventoryResources::new()?;
    let mut runtime = start_inventory(&resources, Mode::Control, 311)?;
    insert_inventory(&mut runtime, 2, true)?;
    let first = true_inventory(&resources, &mut runtime, true)?;
    let (prepared, _) = inventory_plan(&mut runtime, &first, 10)?;
    let mut handoff = resources.recover(&prepared);
    drop(runtime);
    handoff.expected = None;
    assert!(start_inventory(&handoff, Mode::Offline, 312).is_err());
    handoff.expected = prepared
        .inventory_clock
        .as_ref()
        .map(|clock| clock.journal_id);
    handoff.order.expected_journal = None;
    assert!(start_inventory(&handoff, Mode::Offline, 313).is_err());
    Ok(())
}
