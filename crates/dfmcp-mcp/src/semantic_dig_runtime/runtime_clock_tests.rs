//! Joined original excavation, finite native clock, durable proof and recovery.
//! Production owners and codecs are exercised; only I/O and storage are injected.
use super::*;
use dfmcp_adapter::bounded_run::journal::{RunBinding, RunJournal, RunMode, RunState};
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunSource};
use dfmcp_adapter::bounded_run::{RunObservation, RunPhase, RunPlan, RunRecord};
use std::collections::BTreeMap;
use std::net::SocketAddr;

#[derive(Default)]
struct ClockState {
    sequence: u64,
    records: BTreeMap<String, RunRecord>,
    calls: [usize; 5],
    connections: usize,
    lose_commit: bool,
    observed_tick: Option<u64>,
}
#[derive(Clone)]
pub(super) struct ClockResources {
    storage: Memory,
    binding: RunBinding,
    goal: Rc<RefCell<GoalState>>,
    state: Rc<RefCell<ClockState>>,
    enabled: Rc<Cell<bool>>,
    expected_journal: Option<Digest32>,
}
impl ClockResources {
    fn new(goal: Rc<RefCell<GoalState>>) -> Result<Self> {
        Ok(Self {
            storage: Memory::default(),
            binding: RunBinding::new(
                SocketAddr::from(([127, 0, 0, 1], 5000)),
                RunManifest {
                    generation: 901,
                    df_version: "df".into(),
                    dfhack_version: "dfhack".into(),
                },
            )?,
            goal,
            state: Rc::new(RefCell::new(ClockState {
                sequence: 1,
                ..ClockState::default()
            })),
            enabled: Rc::new(Cell::new(true)),
            expected_journal: None,
        })
    }
    pub(super) fn recovered(&self, journal: Digest32) -> Self {
        let mut out = self.clone();
        out.expected_journal = Some(journal);
        out
    }
    pub(super) fn enabled(&self) -> bool {
        self.enabled.get()
    }
    pub(super) fn context(
        &self,
        canonical: &OperationContext,
        control: bool,
    ) -> Result<OperationContext> {
        assert_ne!(canonical.anchor.fortress_id, FortressId::NIL);
        canonical.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if control {
            if !self.enabled() {
                return Err(denied("test clock authority revoked"));
            }
            canonical.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        let mut capabilities = vec![Capability::Query];
        if control {
            capabilities.extend([Capability::Plan, Capability::ControlClock]);
        }
        Ok(OperationContext {
            session_id: canonical.session_id,
            request_id: canonical.request_id,
            anchor: StateAnchor {
                fortress_id: FortressId::NIL,
                cursor: ObservationCursor::ORIGIN,
                tick: canonical.anchor.tick,
                state_hash: Digest32::ZERO,
            },
            budget: canonical.budget,
            grants: capabilities
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
                .collect(),
            cancellation_requested: canonical.cancellation_requested,
        })
    }
    pub(super) fn open(&self, mode: Mode, c: &OperationContext) -> Result<RunJournal<Memory>> {
        let mode = match mode {
            Mode::Offline => RunMode::Offline,
            Mode::Control if self.enabled() => RunMode::Control,
            _ => RunMode::Recover,
        };
        let native = self.context(c, mode == RunMode::Control)?;
        let initialize = self.storage.bytes().is_empty();
        if initialize == self.expected_journal.is_some() {
            return Err(invalid(
                "clock opening requires exact create/recovery identity",
            ));
        }
        let journal = RunJournal::open(
            self.storage.clone(),
            &native,
            mode,
            Some(self.binding.clone()),
            initialize,
        )?;
        if self.expected_journal.is_some_and(|id| id != journal.id()) {
            return Err(invalid("changed external clock journal identity"));
        }
        Ok(journal)
    }
    pub(super) fn source(&self, c: &OperationContext) -> Result<ClockNative> {
        assert_eq!(c.anchor.fortress_id, FortressId::NIL);
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        self.state.borrow_mut().connections += 1;
        Ok(ClockNative(self.clone()))
    }
    fn stop(&self, ticks: u64) -> Result<()> {
        let plan = self
            .state
            .borrow()
            .records
            .values()
            .find(|record| matches!(record.phase(), RunPhase::Running | RunPhase::Stopping))
            .map(|record| record.plan().clone())
            .ok_or_else(|| invalid("running clock absent"))?;
        let tick = {
            let mut goal = self.goal.borrow_mut();
            goal.tick += ticks;
            goal.paused = true;
            goal.tick
        };
        self.state
            .borrow_mut()
            .records
            .insert(plan.key().into(), clock_record(&plan, 3, 1, tick)?);
        Ok(())
    }
    fn durable_state(&self, c: &OperationContext) -> Result<RunState> {
        let native = self.context(c, false)?;
        let mut journal = RunJournal::open(
            Memory::from_bytes(self.storage.durable()),
            &native,
            RunMode::Offline,
            Some(self.binding.clone()),
            false,
        )?;
        journal
            .records(&native)?
            .last()
            .map(|record| record.state())
            .ok_or_else(|| invalid("durable clock absent"))
    }
}
fn clock_record(plan: &RunPlan, phase: u8, reason: u8, tick: u64) -> Result<RunRecord> {
    let mut bytes = b"DFMRE013".to_vec();
    bytes.extend_from_slice(&plan.canonical_bytes());
    bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.token());
    let known = matches!(phase, 1..=3);
    bytes.extend_from_slice(&[
        phase,
        reason,
        u8::from(matches!(phase, 1..=3 | 5)),
        u8::from(phase == 3),
        u8::from(known),
    ]);
    bytes.extend_from_slice(&(if known { tick } else { 0 }).to_be_bytes());
    let mut receipt = b"dfmcp-bounded-run-receipt/1\0".to_vec();
    receipt.extend_from_slice(&bytes);
    bytes.extend_from_slice(Digest32::of_bytes(&receipt).as_bytes());
    RunRecord::decode(&bytes)
}
pub(super) struct ClockNative(ClockResources);
impl ClockNative {
    fn check(&self, c: &OperationContext, control: bool) -> Result<()> {
        assert_eq!(c.anchor.fortress_id, FortressId::NIL);
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if control {
            if !self.0.enabled() {
                return Err(denied("test native clock authority revoked"));
            }
            c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        Ok(())
    }
    fn durable(&self, plan: &RunPlan, state: RunState, c: &OperationContext) -> Result<()> {
        let mut journal = RunJournal::open(
            Memory::from_bytes(self.0.storage.durable()),
            c,
            RunMode::Offline,
            Some(self.0.binding.clone()),
            false,
        )?;
        assert_eq!(
            journal
                .records(c)?
                .iter()
                .find(|record| record.plan() == plan)
                .map(|record| record.state()),
            Some(state)
        );
        Ok(())
    }
}
impl RunSource for ClockNative {
    fn manifest(&self) -> &RunManifest {
        self.0.binding.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(self.0.binding.endpoint())
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        self.check(c, false)?;
        self.0.state.borrow_mut().calls[0] += 1;
        let state = self.0.state.borrow();
        let goal = self.0.goal.borrow();
        let mut bytes = b"DFMRO013".to_vec();
        for value in [
            self.manifest().generation,
            state.sequence,
            state.observed_tick.unwrap_or(goal.tick),
        ] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        bytes.extend_from_slice(&[1, 1, u8::from(goal.paused)]);
        RunObservation::decode(&bytes)
    }
    fn prepare(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        self.durable(p, RunState::Intent, c)?;
        self.0.state.borrow_mut().calls[1] += 1;
        let record = clock_record(p, 0, 0, 0)?;
        self.0
            .state
            .borrow_mut()
            .records
            .insert(p.key().into(), record.clone());
        Ok(record)
    }
    fn commit(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        self.durable(p, RunState::DispatchStarted, c)?;
        let tick = self.0.goal.borrow().tick;
        let record = clock_record(p, 1, 0, tick)?;
        self.0.goal.borrow_mut().paused = false;
        let mut state = self.0.state.borrow_mut();
        state.calls[2] += 1;
        state.sequence += 1;
        state.records.insert(p.key().into(), record.clone());
        if state.lose_commit {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "injected lost unpause reply",
            ));
        }
        Ok(record)
    }
    fn query(&mut self, p: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        self.check(c, false)?;
        self.0.state.borrow_mut().calls[3] += 1;
        Ok(self.0.state.borrow().records.get(p.key()).cloned())
    }
    fn cancel(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.check(c, true)?;
        self.durable(p, RunState::CancelRequested, c)?;
        self.0.state.borrow_mut().calls[4] += 1;
        self.0.goal.borrow_mut().paused = true;
        let tick = self.0.goal.borrow().tick;
        let record = clock_record(p, 3, 3, tick)?;
        self.0
            .state
            .borrow_mut()
            .records
            .insert(p.key().into(), record.clone());
        Ok(record)
    }
}
fn with_clock() -> Result<(Runtime<Host>, Host, ClockResources)> {
    let mut host = Host::new()?;
    let clock = ClockResources::new(host.goal.clone())?;
    host.clock = Some(clock.clone());
    let runtime = host.runtime(Mode::Control)?;
    Ok((runtime, host, clock))
}
fn designated(runtime: &mut Runtime<Host>, stable: u32) -> Result<View> {
    observe_plan(runtime, stable)?;
    commit(runtime)?;
    wait(runtime)?;
    wait(runtime)
}
fn request(view: &View) -> Result<RunRequest> {
    Ok(RunRequest {
        kind: "run".into(),
        original_paired_seal: view
            .review
            .as_ref()
            .ok_or_else(|| invalid("original paired review absent"))?
            .seal()
            .to_hex(),
        game_ticks: 5,
        wall_millis: 1000,
    })
}
fn clock_context(runtime: &mut Runtime<Host>) -> Result<OperationContext> {
    // Host.runtime already applies the actual MCP inspection/output reserves.
    runtime.context(false)
}
fn prepare_clock(runtime: &mut Runtime<Host>, original: &View) -> Result<Digest32> {
    let c = clock_context(runtime)?;
    let view = runtime.prepare_run(request(original)?, &c)?;
    assert_eq!(view.goal.as_ref().map(|goal| goal.anchor), view.anchor);
    assert_eq!(
        view.clock_readiness.as_ref().map(|ready| ready.anchor),
        view.anchor
    );
    assert!(
        view.goal.is_some(),
        "undispatched prepared clock retains its fresh goal for confirmation"
    );
    view.clock
        .as_ref()
        .and_then(|clock| clock.active())
        .map(|run| run.review_seal)
        .ok_or_else(|| invalid("active clock review absent"))
}
fn run_clock(runtime: &mut Runtime<Host>, original: &View) -> Result<View> {
    let seal = prepare_clock(runtime, original)?;
    let c = clock_context(runtime)?;
    runtime.commit_run(seal, true, &c)
}
fn reopen(host: Host, mode: Mode, bytes: u64) -> Result<Runtime<Host>> {
    Runtime::from_shell(
        host,
        mode,
        SessionId::new(772),
        WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: bytes,
            max_entities: 8192,
            max_actions: 1,
            max_output_tokens: MAX_OUTPUT_TOKENS,
            max_game_ticks: MAX_GAME_TICKS,
        },
    )
}
#[test]
fn pending_excavation_advances_with_default_budget_then_original_stability_completes() -> Result<()>
{
    let (mut runtime, host, clock) = with_clock()?;
    host.goal.borrow_mut().condition = "pending";
    let first = designated(&mut runtime, 2)?;
    assert_eq!(
        first.goal.as_ref().map(|g| g.current_goal),
        Some(PredicateTruth::Unknown)
    );
    assert_eq!(
        first.goal.as_ref().map(|g| g.excavation_condition),
        Some("unobserved_or_uncleared")
    );
    let original = first.original.clone();
    let receipt = first.designation.clone();
    let seal = prepare_clock(&mut runtime, &first)?;
    let c = clock_context(&mut runtime)?;
    assert!(runtime.commit_run(seal, false, &c).is_err());
    let running = runtime.commit_run(seal, true, &c)?;
    assert!(running.goal.is_none());
    assert!(running.clock.as_ref().is_some_and(|v| v.may_be_running()));
    let reads = host.goal.borrow().reads;
    assert!(wait(&mut runtime)?.goal.is_none());
    assert_eq!(host.goal.borrow().reads, reads);
    host.goal.borrow_mut().condition = "condition_met";
    clock.stop(5)?;
    let one = wait(&mut runtime)?;
    assert_eq!(
        one.goal.as_ref().map(|g| g.current_goal),
        Some(PredicateTruth::True)
    );
    assert!(!one.goal.as_ref().is_some_and(|g| g.current_goal_proven()));
    run_clock(&mut runtime, &first)?;
    clock.stop(5)?;
    let complete = wait(&mut runtime)?;
    assert!(
        complete
            .goal
            .as_ref()
            .is_some_and(|g| g.current_goal_proven())
    );
    assert_eq!(complete.original, original);
    assert_eq!(complete.designation, receipt);
    let calls = clock.state.borrow().calls;
    let c = clock_context(&mut runtime)?;
    assert!(runtime.prepare_run(request(&first)?, &c).is_err());
    assert_eq!(clock.state.borrow().calls, calls);
    assert_eq!(clock.state.borrow().calls[2], 2);
    assert_eq!(host.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn nonviable_excavation_conditions_never_connect_or_prepare_native_clock() -> Result<()> {
    for condition in [
        "hidden",
        "missing",
        "wet",
        "designated_floor",
        "undesignated_wall",
        "occupied_wall",
        "new_generation",
    ] {
        let (mut runtime, host, clock) = with_clock()?;
        let first = designated(&mut runtime, 2)?;
        host.goal.borrow_mut().condition = condition;
        let c = clock_context(&mut runtime)?;
        assert!(
            runtime.prepare_run(request(&first)?, &c).is_err(),
            "condition {condition}"
        );
        assert_eq!(clock.state.borrow().connections, 0, "condition {condition}");
        assert_eq!(clock.state.borrow().calls, [0; 5]);
        assert_eq!(host.native.borrow().commits, 1);
    }
    Ok(())
}
#[test]
fn post_marker_source_policy_custody_and_clock_revocation_block_single_unpause() -> Result<()> {
    for fault in 0..8 {
        let (mut runtime, host, clock) = with_clock()?;
        host.goal.borrow_mut().condition = "pending";
        let first = designated(&mut runtime, 2)?;
        let seal = prepare_clock(&mut runtime, &first)?;
        let goal = host.goal.clone();
        let archive = host.archive.clone();
        let associations = host.associations.clone();
        let native = host.native_disk.clone();
        let enabled = clock.enabled.clone();
        clock.storage.after_next_sync(move || match fault {
            0 => goal.borrow_mut().condition = "hidden",
            1 => goal.borrow_mut().tick += 1,
            2 => goal.borrow_mut().denied = true,
            3 => archive.corrupt(),
            4 => associations.corrupt(),
            5 => native.lose_identity(),
            6 => enabled.set(false),
            _ => goal.borrow_mut().corrupt_archive = true,
        });
        let c = clock_context(&mut runtime)?;
        assert!(runtime.commit_run(seal, true, &c).is_err(), "fault {fault}");
        assert_eq!(clock.state.borrow().calls[2], 0, "fault {fault}");
        assert_eq!(clock.durable_state(&c)?, RunState::DispatchStarted);
        clock.enabled.set(true);
        let c = clock_context(&mut runtime)?;
        assert!(runtime.commit_run(seal, true, &c).is_err());
        assert_eq!(clock.state.borrow().calls[2], 0);
        assert_eq!(host.native.borrow().commits, 1);
    }
    Ok(())
}
#[test]
fn lost_unpause_reply_recovers_offline_and_reconciles_without_designation_or_unpause_replay()
-> Result<()> {
    let (mut runtime, host, clock) = with_clock()?;
    host.goal.borrow_mut().condition = "pending";
    let first = designated(&mut runtime, 2)?;
    let seal = prepare_clock(&mut runtime, &first)?;
    clock.state.borrow_mut().lose_commit = true;
    let c = clock_context(&mut runtime)?;
    assert!(runtime.commit_run(seal, true, &c).is_err());
    assert_eq!(clock.durable_state(&c)?, RunState::DispatchStarted);
    let c = runtime.context(false)?;
    let retained = runtime.inspect(&c)?;
    assert!(retained.goal.is_none());
    let recovery = host.recovered(&runtime, true)?;
    let calls = clock.state.borrow().calls;
    let connections = clock.state.borrow().connections;
    let excavation_reads = host.goal.borrow().reads;
    drop(runtime);
    let mut offline = reopen(recovery.clone(), Mode::Offline, 8 * 1024 * MIB)?;
    let c = offline.context(false)?;
    let saved = offline.inspect(&c)?;
    assert_eq!(saved.original, first.original);
    assert!(saved.goal.is_none());
    assert_eq!(clock.state.borrow().calls, calls);
    assert_eq!(clock.state.borrow().connections, connections);
    assert_eq!(host.goal.borrow().reads, excavation_reads);
    assert!(offline.wait(&c).is_err());
    drop(offline);
    let mut recovered = reopen(recovery, Mode::Reconcile, 8 * 1024 * MIB)?;
    assert!(wait(&mut recovered)?.goal.is_none());
    host.goal.borrow_mut().condition = "condition_met";
    clock.stop(5)?;
    let current = wait(&mut recovered)?;
    assert_eq!(
        current.goal.as_ref().map(|g| g.current_goal),
        Some(PredicateTruth::True)
    );
    assert!(
        !current
            .goal
            .as_ref()
            .is_some_and(|g| g.current_goal_proven())
    );
    assert!(!recovered.clock_control_enabled());
    assert!(!recovered.designation_control_enabled());
    assert_eq!(clock.state.borrow().calls[2], 1);
    assert_eq!(host.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn settled_clock_tick_and_rejected_capture_floors_prevent_older_goal_publication() -> Result<()> {
    for stopped in [false, true] {
        let (mut runtime, host, clock) = with_clock()?;
        let first = designated(&mut runtime, 2)?;
        let before = host.goal.borrow().tick;
        if stopped {
            run_clock(&mut runtime, &first)?;
            clock.stop(5)?;
            host.goal.borrow_mut().tick = before;
        } else {
            clock.state.borrow_mut().observed_tick = Some(before + 5);
            let c = clock_context(&mut runtime)?;
            assert!(runtime.prepare_run(request(&first)?, &c).is_err());
        }
        assert!(wait(&mut runtime).is_err());
        let c = runtime.context(false)?;
        assert!(c.anchor.tick.get() >= before + 5);
        let current = runtime.inspect(&c)?;
        assert!(current.goal.is_none());
        assert_eq!(clock.state.borrow().calls[2], usize::from(stopped));
        assert_eq!(host.native.borrow().commits, 1);
    }
    Ok(())
}
#[test]
fn safety_pause_after_deadline_needs_no_plan_observe_or_excavation_grant() -> Result<()> {
    let (mut runtime, host, clock) = with_clock()?;
    host.goal.borrow_mut().condition = "pending";
    let first = designated(&mut runtime, 2)?;
    run_clock(&mut runtime, &first)?;
    let deadline = first
        .original
        .as_ref()
        .ok_or_else(|| invalid("original absent"))?
        .steps[0]
        .obligation
        .as_ref()
        .ok_or_else(|| invalid("obligation absent"))?
        .deadline_tick
        .get();
    host.enabled.set(false);
    host.goal.borrow_mut().tick = deadline + 100;
    host.goal.borrow_mut().denied = true;
    let mut c = runtime.context(false)?;
    c.grants.retain(|grant| {
        matches!(
            grant.capability,
            Capability::Query | Capability::ControlClock
        )
    });
    c.budget.max_game_ticks = 0;
    let reads = host.goal.borrow().reads;
    let cancelled = runtime.cancel_run(&c)?;
    assert!(cancelled.goal.is_none());
    assert!(!cancelled.clock.as_ref().is_some_and(|v| v.may_be_running()));
    assert_eq!(host.goal.borrow().reads, reads);
    assert_eq!(clock.state.borrow().calls[4], 1);
    assert!(host.goal.borrow().paused);
    assert_eq!(host.native.borrow().commits, 1);
    assert_eq!(runtime.context(false)?.anchor.tick.get(), deadline + 100);
    Ok(())
}
#[test]
fn clock_only_control_recovery_never_regains_designation_or_latches_unfinished_stability()
-> Result<()> {
    let (mut runtime, host, clock) = with_clock()?;
    host.goal.borrow_mut().tick += 5;
    let first = designated(&mut runtime, 2)?;
    assert!(matches!(
        first.goal.as_ref().and_then(|g| g.obligation.as_ref()),
        Some(ObligationStatus::Active {
            consecutive_stable_observations: 1,
            ..
        })
    ));
    let recovery = host.recovered(&runtime, true)?;
    drop(runtime);
    host.enabled.set(false);
    let mut recovered = reopen(recovery, Mode::Control, DEFAULT_CLOCK_BYTES)?;
    assert!(!recovered.designation_control_enabled());
    assert!(recovered.clock_control_enabled());
    let c = recovered.context(false)?;
    assert!(
        !c.grants
            .iter()
            .any(|g| g.capability == Capability::Designate)
    );
    assert!(recovered.inspect(&c)?.goal.is_none());
    let current = wait(&mut recovered)?;
    assert!(
        !current
            .goal
            .as_ref()
            .is_some_and(|g| g.current_goal_proven())
    );
    run_clock(&mut recovered, &first)?;
    clock.stop(5)?;
    let restarted = wait(&mut recovered)?;
    assert!(matches!(
        restarted.goal.as_ref().and_then(|g| g.obligation.as_ref()),
        Some(ObligationStatus::Active {
            consecutive_stable_observations: 1,
            ..
        })
    ));
    run_clock(&mut recovered, &first)?;
    clock.stop(5)?;
    let complete = wait(&mut recovered)?;
    assert!(
        complete
            .goal
            .as_ref()
            .is_some_and(|g| g.current_goal_proven())
    );
    assert_eq!(host.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn actual_clock_target_binds_original_archive_review_deadline_and_designated_receipt() -> Result<()>
{
    let (mut runtime, _, _) = with_clock()?;
    let first = designated(&mut runtime, 2)?;
    let plan = first
        .original
        .as_ref()
        .ok_or_else(|| invalid("original absent"))?;
    let review = first
        .review
        .as_ref()
        .ok_or_else(|| invalid("review absent"))?;
    let native = first
        .native
        .as_ref()
        .ok_or_else(|| invalid("receipt absent"))?;
    let archive = first.archive.ok_or_else(|| invalid("archive absent"))?;
    let target = ClockTarget::new(plan, archive, first.journal, review, native)?;
    assert_eq!(target.original_paired_seal(), review.seal());
    assert_eq!(
        target.deadline_tick(),
        plan.steps[0]
            .obligation
            .as_ref()
            .ok_or_else(|| invalid("obligation absent"))?
            .deadline_tick
            .get()
    );
    let mut changed = archive;
    changed.object_digest = Digest32::of_bytes(b"different original object");
    assert!(ClockTarget::new(plan, changed, first.journal, review, native).is_err());
    let mut changed = plan.clone();
    changed.summary.push('x');
    assert!(ClockTarget::new(&changed, archive, first.journal, review, native).is_err());
    assert!(ClockTarget::new(plan, archive, Digest32::ZERO, review, native).is_err());
    Ok(())
}
#[test]
fn sixty_four_stable_samples_fit_finite_runs_and_bounded_exact_recovery() -> Result<()> {
    let (mut runtime, host, clock) = with_clock()?;
    // Request a larger explicit logical allowance for long complete-byte history verification.
    runtime.budget.max_bytes = MAX_BYTES;
    let first = designated(&mut runtime, 64)?;
    let first_tick = host.goal.borrow().tick;
    let mut complete = first.clone();
    for sample in 1..=64 {
        run_clock(&mut runtime, &first)?;
        clock.stop(5)?;
        complete = wait(&mut runtime)?;
        assert_eq!(
            complete
                .goal
                .as_ref()
                .is_some_and(|g| g.current_goal_proven()),
            sample == 64
        );
    }
    assert_eq!(
        complete.first_satisfied_anchor.map(|a| a.tick),
        Some(GameTick(first_tick + 64 * 5))
    );
    assert_eq!(complete.clock.as_ref().map(|v| v.runs.len()), Some(64));
    assert!(complete.history_events > 512);
    assert!(complete.history_events < 1024);
    assert_eq!(complete.original, first.original);
    assert_eq!(complete.designation, first.designation);
    let recovery = host.recovered(&runtime, true)?;
    let history = host.history.durable();
    let calls = clock.state.borrow().calls;
    let connections = clock.state.borrow().connections;
    let reads = host.goal.borrow().reads;
    drop(runtime);
    let insufficient = reopen(recovery.clone(), Mode::Offline, 32 * 1024 * MIB);
    assert!(matches!(insufficient,Err(cause) if cause.code==ErrorCode::BudgetExceeded));
    assert_eq!(host.history.durable(), history);
    let mut recovered = reopen(recovery, Mode::Offline, MAX_BYTES)?;
    let c = recovered.context(false)?;
    let retained = recovered.inspect(&c)?;
    assert_eq!(
        retained.first_satisfied_anchor,
        complete.first_satisfied_anchor
    );
    assert_eq!(retained.original, complete.original);
    assert_eq!(retained.history_events, complete.history_events);
    assert_eq!(retained.clock.as_ref().map(|v| v.runs.len()), Some(64));
    assert!(retained.goal.is_none());
    assert_eq!(clock.state.borrow().calls, calls);
    assert_eq!(clock.state.borrow().connections, connections);
    assert_eq!(host.goal.borrow().reads, reads);
    assert_eq!(host.history.durable(), history);
    assert_eq!(clock.state.borrow().calls[2], 64);
    assert_eq!(host.native.borrow().commits, 1);
    Ok(())
}

#[test]
fn denied_policy_retains_private_cursor_for_safety_pause_and_fresh_retry() -> Result<()> {
    for at_clock_edge in [false, true] {
        let (mut runtime, host, clock) = with_clock()?;
        host.goal.borrow_mut().condition = "pending";
        let first = designated(&mut runtime, 2)?;
        if at_clock_edge {
            let seal = prepare_clock(&mut runtime, &first)?;
            let goal = host.goal.clone();
            clock
                .storage
                .after_next_sync(move || goal.borrow_mut().denied = true);
            let c = clock_context(&mut runtime)?;
            assert!(runtime.commit_run(seal, true, &c).is_err());
            assert_eq!(clock.state.borrow().calls[2], 0);
        } else {
            host.goal.borrow_mut().denied = true;
            assert!(wait(&mut runtime).is_err());
        }
        let published = runtime
            .anchor()
            .ok_or_else(|| invalid("last publication absent"))?;
        let mut c = runtime.context(false)?;
        assert!(c.anchor.cursor.sequence > published.cursor.sequence);
        assert_eq!(c.anchor.tick, published.tick);
        host.goal.borrow_mut().denied = false;
        if at_clock_edge {
            c.grants.retain(|grant| {
                matches!(
                    grant.capability,
                    Capability::Query | Capability::ControlClock
                )
            });
            let cancelled = runtime.cancel_run(&c)?;
            assert!(cancelled.goal.is_none());
            assert_eq!(cancelled.anchor, Some(published));
            assert_eq!(clock.state.borrow().calls[4], 1);
        }
        let current = wait(&mut runtime)?;
        let goal = current
            .goal
            .as_ref()
            .ok_or_else(|| invalid("fresh pending goal absent"))?;
        assert_eq!(goal.current_goal, PredicateTruth::Unknown);
        assert!(goal.anchor.cursor.sequence > c.anchor.cursor.sequence);
        assert_eq!(current.original, first.original);
        assert_eq!(clock.state.borrow().calls[2], 0);
        assert_eq!(host.native.borrow().commits, 1);
    }
    Ok(())
}

#[test]
fn underfunded_complete_clock_work_refuses_before_native_clock_contact() -> Result<()> {
    let (mut runtime, host, clock) = with_clock()?;
    host.goal.borrow_mut().condition = "pending";
    let first = designated(&mut runtime, 2)?;
    let mut c = clock_context(&mut runtime)?;
    c.budget.max_bytes = DEFAULT_BYTES - 256 * MIB - 128 * 1024;
    assert!(
        matches!(runtime.prepare_run(request(&first)?, &c),Err(cause) if cause.code==ErrorCode::BudgetExceeded)
    );
    assert_eq!(clock.state.borrow().connections, 0);
    assert_eq!(clock.state.borrow().calls, [0; 5]);
    let seal = prepare_clock(&mut runtime, &first)?;
    let before_calls = clock.state.borrow().calls;
    let before_connections = clock.state.borrow().connections;
    let mut c = clock_context(&mut runtime)?;
    c.budget.max_bytes = DEFAULT_BYTES - 256 * MIB - 128 * 1024;
    assert!(
        matches!(runtime.commit_run(seal,true,&c),Err(cause) if cause.code==ErrorCode::BudgetExceeded)
    );
    assert_eq!(clock.state.borrow().calls, before_calls);
    assert_eq!(clock.state.borrow().connections, before_connections);
    assert_eq!(clock.state.borrow().calls[2], 0);
    let c = clock_context(&mut runtime)?;
    assert!(runtime.commit_run(seal, true, &c)?.goal.is_none());
    assert_eq!(clock.state.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn raw_clock_qualification_is_independent_and_never_upgrades_unknown_goal_proof() -> Result<()> {
    let (mut runtime, host, clock) = with_clock()?;
    host.goal.borrow_mut().condition = "pending";
    host.clock_terrain_enabled.set(false);
    let first = designated(&mut runtime, 2)?;
    assert_eq!(
        first.goal.as_ref().map(|g| g.current_goal),
        Some(PredicateTruth::Unknown)
    );
    assert!(!first.clock_readiness.as_ref().is_some_and(|r| r.eligible));
    let c = clock_context(&mut runtime)?;
    assert!(runtime.prepare_run(request(&first)?, &c).is_err());
    assert_eq!(clock.state.borrow().connections, 0);
    host.clock_terrain_enabled.set(true);
    host.goal.borrow_mut().condition = "mixed";
    let mixed = wait(&mut runtime)?;
    let readiness = mixed
        .clock_readiness
        .as_ref()
        .ok_or_else(|| invalid("fresh raw qualification absent"))?;
    assert!(readiness.eligible);
    assert!(readiness.pending_tiles > 0);
    assert!(readiness.cleared_tiles > 0);
    assert_eq!(
        mixed.goal.as_ref().map(|g| g.current_goal),
        Some(PredicateTruth::Unknown)
    );
    assert!(!mixed.goal.as_ref().is_some_and(|g| g.current_goal_proven()));
    assert_eq!(
        mixed.to_json()["clock_readiness"]["original_goal_truth_upgraded"],
        false
    );
    let seal = prepare_clock(&mut runtime, &first)?;
    let c = clock_context(&mut runtime)?;
    assert!(runtime.commit_run(seal, true, &c)?.goal.is_none());
    assert_eq!(clock.state.borrow().calls[2], 1);
    assert_eq!(host.native.borrow().commits, 1);
    Ok(())
}
