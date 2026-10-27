//! Joined original-goal, native clock, durable custody and recovery regressions.
//! Only external source/storage edges are injected; both coordinators, the
//! original archive and temporal goal monitor are the production implementations.
use super::*;
use crate::semantic_workforce_runtime::clock::RunRequest;
use dfmcp_adapter::bounded_run::journal::{RunBinding, RunJournal, RunMode, RunState};
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunSource};
use dfmcp_adapter::bounded_run::{RunObservation, RunPhase, RunPlan, RunRecord};
use dfmcp_core::{CapabilityScope, FortressId, StateAnchor};
use std::collections::BTreeMap;

#[derive(Default)]
struct ClockState {
    sequence: u64,
    records: BTreeMap<String, RunRecord>,
    calls: [usize; 5],
    connections: usize,
    lose_commit: bool,
}
#[derive(Clone)]
pub(super) struct ClockResources {
    storage: Storage,
    binding: RunBinding,
    workforce: Native,
    state: Rc<RefCell<ClockState>>,
    enabled: Rc<Cell<bool>>,
    expected_journal: Option<Digest32>,
}
impl ClockResources {
    fn new(workforce: Native) -> Result<Self> {
        Ok(Self {
            storage: Storage::default(),
            binding: RunBinding::new(
                SocketAddr::from(([127, 0, 0, 1], 5000)),
                RunManifest {
                    generation: 901,
                    df_version: "0.51.11".into(),
                    dfhack_version: "0.51.11-r1".into(),
                },
            )?,
            workforce,
            state: Rc::new(RefCell::new(ClockState {
                sequence: 1,
                ..ClockState::default()
            })),
            enabled: Rc::new(Cell::new(true)),
            expected_journal: None,
        })
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
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "clock authority revoked",
                ));
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
    pub(super) fn open(&self, mode: Mode, c: &OperationContext) -> Result<RunJournal<Storage>> {
        let mode = match mode {
            Mode::Offline => RunMode::Offline,
            Mode::Control if self.enabled() => RunMode::Control,
            _ => RunMode::Recover,
        };
        let native = self.context(c, mode == RunMode::Control)?;
        let initialize = self.storage.0.borrow().file.get_ref().is_empty();
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
    fn recover(&self, view: &View) -> Result<Self> {
        let mut next = self.clone();
        next.expected_journal = Some(
            view.clock
                .as_ref()
                .ok_or_else(|| invalid("clock handoff absent"))?
                .journal_id,
        );
        Ok(next)
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
            let mut workforce = self.workforce.data.borrow_mut();
            workforce.fixture.tick += ticks;
            workforce.fixture.paused = true;
            workforce.fixture.tick
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
            Storage::from_bytes(self.storage.0.borrow().synced.clone()),
            &native,
            RunMode::Offline,
            Some(self.binding.clone()),
            false,
        )?;
        journal
            .records(&native)?
            .last()
            .map(|record| record.state())
            .ok_or_else(|| invalid("durable clock record absent"))
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
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "clock source authority revoked",
                ));
            }
            c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        Ok(())
    }
    fn durable(&self, plan: &RunPlan, state: RunState, c: &OperationContext) -> Result<()> {
        let mut journal = RunJournal::open(
            Storage::from_bytes(self.0.storage.0.borrow().synced.clone()),
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
        let workforce = self.0.workforce.data.borrow();
        let mut bytes = b"DFMRO013".to_vec();
        for value in [
            self.manifest().generation,
            self.0.state.borrow().sequence,
            workforce.fixture.tick,
        ] {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        bytes.extend_from_slice(&[1, 1, u8::from(workforce.fixture.paused)]);
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
        let tick = self.0.workforce.data.borrow().fixture.tick;
        let record = clock_record(p, 1, 0, tick)?;
        self.0.workforce.data.borrow_mut().fixture.paused = false;
        let mut state = self.0.state.borrow_mut();
        state.calls[2] += 1;
        state.sequence += 1;
        state.records.insert(p.key().into(), record.clone());
        if state.lose_commit {
            return Err(error(
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
        self.0.workforce.data.borrow_mut().fixture.paused = true;
        let tick = self.0.workforce.data.borrow().fixture.tick;
        let record = clock_record(p, 3, 3, tick)?;
        self.0
            .state
            .borrow_mut()
            .records
            .insert(p.key().into(), record.clone());
        Ok(record)
    }
}

fn with_clock() -> Result<(Runtime<MockShell>, MockShell, ClockResources)> {
    let (runtime, mut shell) = runtime_fixture()?;
    let mut budget = runtime.budget();
    budget.max_game_ticks = 1200;
    drop(runtime);
    let clock = ClockResources::new(shell.native.clone())?;
    shell.clock = Some(clock.clone());
    let runtime = Runtime::from_shell(shell.clone(), Mode::Control, SessionId::new(101), budget)?;
    Ok((runtime, shell, clock))
}
fn assign(runtime: &mut Runtime<MockShell>, shell: &MockShell) -> Result<View> {
    let prepared = prepare_runtime(runtime)?;
    let paired = prepared
        .review
        .as_ref()
        .ok_or_else(|| invalid("review absent"))?
        .seal();
    let c = runtime.context(false)?;
    runtime.commit(paired, true, &c)?;
    capture_after(shell, 125, true);
    let c = runtime.context(false)?;
    runtime.wait(&c)
}
fn request(view: &View) -> Result<RunRequest> {
    Ok(RunRequest {
        kind: "run".into(),
        original_paired_seal: view
            .review
            .as_ref()
            .ok_or_else(|| invalid("paired review absent"))?
            .seal()
            .to_hex(),
        game_ticks: 5,
        wall_millis: 1000,
    })
}
fn prepare_clock(runtime: &mut Runtime<MockShell>, original: &View) -> Result<Digest32> {
    let c = runtime.context(false)?;
    let view = runtime.prepare_run(request(original)?, &c)?;
    view.clock
        .as_ref()
        .and_then(|clock| clock.active())
        .map(|run| run.review_seal)
        .ok_or_else(|| invalid("active clock review absent"))
}
fn recovered_shell(shell: &MockShell, clock: &ClockResources, view: &View) -> Result<MockShell> {
    let mut next = shell.clone();
    next.recovery = Some(recovery(view)?);
    next.clock = Some(clock.recover(view)?);
    Ok(next)
}

#[test]
fn explicit_finite_run_only_completes_after_fresh_original_stability_sample() -> Result<()> {
    let (mut runtime, shell, clock) = with_clock()?;
    let first = assign(&mut runtime, &shell)?;
    assert_eq!(first.to_json()["current_goal"]["current_truth"], "true");
    assert_eq!(
        first.to_json()["current_goal"]["original_goal_proven"],
        false
    );
    let original = first.original.clone();
    let seal = prepare_clock(&mut runtime, &first)?;
    let c = runtime.context(false)?;
    assert!(runtime.commit_run(seal, false, &c).is_err());
    assert_eq!(clock.state.borrow().calls[2], 0);
    let running = runtime.commit_run(seal, true, &c)?;
    assert!(running.clock.as_ref().is_some_and(|v| v.may_be_running()));
    assert_eq!(running.to_json()["clock"]["original_goal_proven"], false);
    assert!(running.first_satisfied_anchor.is_none());
    clock.stop(5)?;
    let c = runtime.context(false)?;
    let complete = runtime.wait(&c)?;
    assert_eq!(complete.original, original);
    assert_eq!(
        complete.to_json()["current_goal"]["semantic_completion_proven"],
        true
    );
    assert_eq!(
        complete.first_satisfied_anchor.map(|a| a.tick),
        Some(GameTick(130))
    );
    assert_eq!(
        complete.to_json()["clock"]["runs"][0]["native"]["historical_pause_verified"],
        true
    );
    assert_eq!(
        complete.to_json()["clock"]["runs"][0]["native"]["original_goal_proven"],
        false
    );
    let calls = clock.state.borrow().calls;
    let c = runtime.context(false)?;
    assert!(runtime.prepare_run(request(&first)?, &c).is_err());
    assert_eq!(clock.state.borrow().calls, calls);
    assert_eq!(clock.state.borrow().calls[2], 1);
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn running_clock_wait_reconciles_without_sampling_or_reusing_current_goal() -> Result<()> {
    let (mut runtime, shell, clock) = with_clock()?;
    let first = assign(&mut runtime, &shell)?;
    let seal = prepare_clock(&mut runtime, &first)?;
    let c = runtime.context(false)?;
    let running = runtime.commit_run(seal, true, &c)?;
    let calls = shell.native.data.borrow().calls;
    let history = shell.history.0.borrow().synced.clone();
    let queries = clock.state.borrow().calls[3];
    let c = runtime.context(false)?;
    let waiting = runtime.wait(&c)?;
    assert_eq!(clock.state.borrow().calls[3], queries + 1);
    assert_eq!(shell.native.data.borrow().calls, calls);
    assert_eq!(shell.history.0.borrow().synced, history);
    assert_eq!(waiting.history_events, running.history_events);
    assert!(waiting.clock.as_ref().is_some_and(|v| v.may_be_running()));
    assert!(waiting.goal.is_none());
    assert_eq!(waiting.to_json()["current_goal_available"], false);
    let c = runtime.context(false)?;
    assert!(runtime.commit_run(seal, true, &c).is_err());
    assert_eq!(clock.state.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn lost_unpause_reply_recovers_offline_then_queries_without_redispatch() -> Result<()> {
    let (mut runtime, shell, clock) = with_clock()?;
    let first = assign(&mut runtime, &shell)?;
    let seal = prepare_clock(&mut runtime, &first)?;
    clock.state.borrow_mut().lose_commit = true;
    let c = runtime.context(false)?;
    assert!(runtime.commit_run(seal, true, &c).is_err());
    assert_eq!(clock.durable_state(&c)?, RunState::DispatchStarted);
    let c = runtime.context(false)?;
    let uncertain = runtime.inspect(&c)?;
    let reopen = recovered_shell(&shell, &clock, &uncertain)?;
    let mut budget = runtime.budget();
    budget.max_bytes = 8 * 1024 * 1024 * 1024;
    let calls = clock.state.borrow().calls;
    let workforce_calls = shell.native.data.borrow().calls;
    let connections = clock.state.borrow().connections;
    drop(runtime);
    let mut offline =
        Runtime::from_shell(reopen.clone(), Mode::Offline, SessionId::new(102), budget)?;
    let c = offline.context(false)?;
    let retained = offline.wait(&c)?;
    assert_eq!(retained.original, first.original);
    assert!(retained.goal.is_none());
    assert!(retained.clock.as_ref().is_some_and(|v| v.unresolved));
    assert_eq!(clock.state.borrow().calls, calls);
    assert_eq!(clock.state.borrow().connections, connections);
    assert_eq!(shell.native.data.borrow().calls, workforce_calls);
    assert!(offline.commit_run(seal, true, &c).is_err());
    drop(offline);
    let mut reconciled = Runtime::from_shell(reopen, Mode::Reconcile, SessionId::new(103), budget)?;
    let c = reconciled.context(false)?;
    let running = reconciled.wait(&c)?;
    assert!(running.clock.as_ref().is_some_and(|v| v.may_be_running()));
    assert!(running.goal.is_none());
    clock.stop(5)?;
    let c = reconciled.context(false)?;
    let current = reconciled.wait(&c)?;
    assert_eq!(current.to_json()["current_goal"]["current_truth"], "true");
    assert_eq!(
        current.to_json()["current_goal"]["original_goal_proven"],
        false
    );
    assert!(!reconciled.clock_control_enabled());
    assert!(!reconciled.labor_control_enabled());
    assert_eq!(clock.state.borrow().calls[2], 1);
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn clock_revocation_after_durable_marker_prevents_unpause_and_never_retries() -> Result<()> {
    let (mut runtime, shell, clock) = with_clock()?;
    let first = assign(&mut runtime, &shell)?;
    let seal = prepare_clock(&mut runtime, &first)?;
    let enabled = Rc::clone(&clock.enabled);
    let next = clock.storage.0.borrow().syncs + 1;
    clock.storage.0.borrow_mut().on_sync = Some((next, Rc::new(move || enabled.set(false))));
    let c = runtime.context(false)?;
    assert!(runtime.commit_run(seal, true, &c).is_err());
    assert_eq!(clock.state.borrow().calls[2], 0);
    assert_eq!(clock.durable_state(&c)?, RunState::DispatchStarted);
    clock.enabled.set(true);
    let c = runtime.context(false)?;
    assert!(runtime.commit_run(seal, true, &c).is_err());
    assert_eq!(clock.state.borrow().calls[2], 0);
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    Ok(())
}

#[test]
fn original_plan_or_identity_damage_after_clock_marker_blocks_unpause() -> Result<()> {
    for damage_identity in [false, true] {
        let (mut runtime, shell, clock) = with_clock()?;
        let first = assign(&mut runtime, &shell)?;
        let seal = prepare_clock(&mut runtime, &first)?;
        let original = if damage_identity {
            shell.identity.clone()
        } else {
            shell.archive.clone()
        };
        let next = clock.storage.0.borrow().syncs + 1;
        clock.storage.0.borrow_mut().on_sync = Some((
            next,
            Rc::new(move || {
                original.0.borrow_mut().file.get_mut()[0] ^= 1;
            }),
        ));
        let c = runtime.context(false)?;
        assert!(runtime.commit_run(seal, true, &c).is_err());
        assert_eq!(clock.state.borrow().calls[2], 0);
        assert_eq!(clock.durable_state(&c)?, RunState::DispatchStarted);
        let c = runtime.context(false)?;
        assert!(runtime.commit_run(seal, true, &c).is_err());
        assert_eq!(clock.state.borrow().calls[2], 0);
    }
    Ok(())
}

#[test]
fn recovered_clock_only_control_cannot_regain_assignment_authority() -> Result<()> {
    let (mut runtime, shell, clock) = with_clock()?;
    let first = assign(&mut runtime, &shell)?;
    let reopen = recovered_shell(&shell, &clock, &first)?;
    let budget = runtime.budget();
    drop(runtime);
    shell.enabled.set(false);
    let mut recovered = Runtime::from_shell(reopen, Mode::Control, SessionId::new(104), budget)?;
    shell.enabled.set(true);
    assert!(recovered.clock_control_enabled());
    assert!(!recovered.labor_control_enabled());
    let seal = prepare_clock(&mut recovered, &first)?;
    let c = recovered.context(false)?;
    recovered.commit_run(seal, true, &c)?;
    clock.stop(5)?;
    let c = recovered.context(false)?;
    let resumed = recovered.wait(&c)?;
    // Reopening deliberately discards the unfinished streak. The retained
    // cadence floor also prevents same-tick preparation checks from counting
    // as a new stability sample, so two later ticks are still necessary.
    assert_eq!(resumed.to_json()["current_goal"]["current_truth"], "true");
    assert_eq!(
        resumed.to_json()["current_goal"]["semantic_completion_proven"],
        false
    );
    let seal = prepare_clock(&mut recovered, &first)?;
    let c = recovered.context(false)?;
    recovered.commit_run(seal, true, &c)?;
    clock.stop(5)?;
    let c = recovered.context(false)?;
    let complete = recovered.wait(&c)?;
    assert_eq!(
        complete.to_json()["current_goal"]["semantic_completion_proven"],
        true
    );
    let paired = first
        .review
        .as_ref()
        .ok_or_else(|| invalid("paired review absent"))?
        .seal();
    let c = recovered.context(false)?;
    assert!(recovered.commit(paired, true, &c).is_err());
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    assert_eq!(clock.state.borrow().calls[2], 2);
    Ok(())
}

#[test]
fn native_stop_tick_fences_regressing_labor_publication() -> Result<()> {
    let (mut runtime, shell, clock) = with_clock()?;
    let first = assign(&mut runtime, &shell)?;
    let seal = prepare_clock(&mut runtime, &first)?;
    let c = runtime.context(false)?;
    runtime.commit_run(seal, true, &c)?;
    clock.stop(5)?;
    capture_after(&shell, 129, true);
    let c = runtime.context(false)?;
    assert!(runtime.wait(&c).is_err());
    assert_eq!(clock.state.borrow().calls[2], 1);
    capture_after(&shell, 130, true);
    let c = runtime.context(false)?;
    let fresh = runtime.wait(&c)?;
    assert_eq!(fresh.anchor.tick, GameTick(130));
    assert_eq!(fresh.to_json()["current_goal"]["current_truth"], "true");
    assert_eq!(
        fresh.to_json()["current_goal"]["original_goal_proven"],
        false
    );
    assert!(fresh.first_satisfied_anchor.is_none());
    Ok(())
}

#[test]
fn safety_pause_after_original_deadline_needs_no_plan_observe_or_labor_grant() -> Result<()> {
    let (mut runtime, shell, clock) = with_clock()?;
    let first = assign(&mut runtime, &shell)?;
    let seal = prepare_clock(&mut runtime, &first)?;
    let c = runtime.context(false)?;
    runtime.commit_run(seal, true, &c)?;
    shell.native.data.borrow_mut().fixture.tick = 500;
    let reads = shell.native.data.borrow().calls;
    let mut c = runtime.context(false)?;
    c.grants
        .retain(|g| matches!(g.capability, Capability::Query | Capability::ControlClock));
    let stopped = runtime.cancel_run(&c)?;
    assert_eq!(clock.state.borrow().calls[4], 1);
    assert_eq!(clock.state.borrow().calls[2], 1);
    assert_eq!(shell.native.data.borrow().calls, reads);
    assert!(!stopped.clock.as_ref().is_some_and(|v| v.may_be_running()));
    assert_eq!(
        stopped.to_json()["clock"]["runs"][0]["native"]["observed_tick"],
        500
    );
    assert_eq!(stopped.original, first.original);
    assert!(stopped.first_satisfied_anchor.is_none());
    Ok(())
}

#[test]
fn sixty_four_samples_fit_explicit_clock_runs_and_exact_offline_recovery() -> Result<()> {
    let (mut runtime, shell, clock) = with_clock()?;
    let c = runtime.context(false)?;
    let observed = runtime.observe(&c)?;
    let mut request = request_for(&observed)?;
    request.deadline_tick = 600;
    request.stable_samples = 64;
    let c = runtime.context(false)?;
    let prepared = runtime.plan(request, &c)?;
    let paired = prepared
        .review
        .as_ref()
        .ok_or_else(|| invalid("paired review absent"))?
        .seal();
    let c = runtime.context(false)?;
    runtime.commit(paired, true, &c)?;
    capture_after(&shell, 125, true);
    let c = runtime.context(false)?;
    let original = runtime.wait(&c)?;
    let mut complete = original.clone();
    for sample in 2..=64 {
        let seal = prepare_clock(&mut runtime, &original)?;
        let c = runtime.context(false)?;
        runtime.commit_run(seal, true, &c)?;
        clock.stop(5)?;
        let c = runtime.context(false)?;
        complete = runtime.wait(&c)?;
        assert_eq!(
            complete.to_json()["current_goal"]["semantic_completion_proven"],
            sample == 64
        );
    }
    assert_eq!(
        complete.first_satisfied_anchor.map(|a| a.tick),
        Some(GameTick(440))
    );
    assert_eq!(complete.clock.as_ref().map(|v| v.runs.len()), Some(63));
    assert!(complete.history_events < 1024);
    let reopen = recovered_shell(&shell, &clock, &complete)?;
    let mut budget = runtime.budget();
    budget.max_bytes = super::super::MAX_BYTES;
    let calls = clock.state.borrow().calls;
    let workforce_calls = shell.native.data.borrow().calls;
    let history = shell.history.0.borrow().synced.clone();
    drop(runtime);
    let mut offline = Runtime::from_shell(reopen, Mode::Offline, SessionId::new(105), budget)?;
    let c = offline.context(false)?;
    let recovered = offline.inspect(&c)?;
    assert_eq!(recovered.original, complete.original);
    assert_eq!(
        recovered.first_satisfied_anchor,
        complete.first_satisfied_anchor
    );
    assert_eq!(recovered.clock.as_ref().map(|v| v.runs.len()), Some(63));
    assert_eq!(recovered.history_events, complete.history_events);
    assert!(recovered.goal.is_none());
    assert_eq!(clock.state.borrow().calls, calls);
    assert_eq!(shell.native.data.borrow().calls, workforce_calls);
    assert_eq!(shell.history.0.borrow().synced, history);
    assert_eq!(clock.state.borrow().calls[2], 63);
    assert_eq!(shell.native.data.borrow().calls[2], 1);
    Ok(())
}
