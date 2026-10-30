//! Whole-blueprint handlers exercise real run journals and typed native evidence.
//! Designation, terrain and clock sources have independent generations.
use super::*;
use dfmcp_adapter::bounded_run::journal::{RunBinding, RunJournal, RunMode, RunState};
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunSource};
use dfmcp_adapter::bounded_run::{RunObservation, RunPlan, RunRecord};
use dfmcp_adapter::dig_blueprint::clock::{Clock as ClockOwner, Recovery as ClockRecovery};
use dfmcp_core::FortressId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Observe,
    Prepare,
    Commit,
    Query,
    Cancel,
}
struct ClockGame {
    tick: u64,
    sequence: u64,
    paused: bool,
    calls: Vec<Call>,
    plans: BTreeMap<String, RunPlan>,
    records: BTreeMap<String, RunRecord>,
    last: Option<RunPlan>,
    lose_commit: bool,
    absent: bool,
}
impl Default for ClockGame {
    fn default() -> Self {
        Self {
            tick: TICK,
            sequence: 0,
            paused: true,
            calls: vec![],
            plans: BTreeMap::new(),
            records: BTreeMap::new(),
            last: None,
            lose_commit: false,
            absent: false,
        }
    }
}
impl ClockGame {
    fn count(&self, call: Call) -> usize {
        self.calls.iter().filter(|value| **value == call).count()
    }
    fn observe(&self) -> Result<RunObservation> {
        let mut raw = b"DFMRO013".to_vec();
        for value in [41, self.sequence, self.tick] {
            raw.extend_from_slice(&value.to_be_bytes());
        }
        raw.extend_from_slice(&[1, 1, u8::from(self.paused)]);
        RunObservation::decode(&raw)
    }
    fn settle(&mut self, tick: u64) -> Result<()> {
        let plan = self.last.clone().ok_or_else(exhausted)?;
        let record = native_record(&plan, 3, 1, true, true, Some(tick))?;
        self.records.insert(plan.key().to_owned(), record);
        self.tick = tick;
        self.paused = true;
        Ok(())
    }
    fn lose_source(&mut self) -> Result<()> {
        let plan = self.last.clone().ok_or_else(exhausted)?;
        let record = native_record(&plan, 5, 7, true, false, None)?;
        self.records.insert(plan.key().to_owned(), record);
        self.paused = false;
        Ok(())
    }
}
fn native_record(
    plan: &RunPlan,
    phase: u8,
    reason: u8,
    attempted: bool,
    verified: bool,
    tick: Option<u64>,
) -> Result<RunRecord> {
    let mut raw = b"DFMRE013".to_vec();
    raw.extend_from_slice(&plan.canonical_bytes());
    raw.extend_from_slice(plan.digest().as_bytes());
    raw.extend_from_slice(plan.token());
    raw.extend_from_slice(&[
        phase,
        reason,
        u8::from(attempted),
        u8::from(verified),
        u8::from(tick.is_some()),
    ]);
    raw.extend_from_slice(&tick.map_or(0, |value| value).to_be_bytes());
    raw.extend_from_slice(hash(b"dfmcp-bounded-run-receipt/1", &raw).as_bytes());
    RunRecord::decode(&raw)
}
struct ClockNative {
    game: Rc<RefCell<ClockGame>>,
    manifest: RunManifest,
    storage: Memory,
    endpoint: std::net::SocketAddr,
}
impl RunSource for ClockNative {
    fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<std::net::SocketAddr> {
        Some(self.endpoint)
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        assert_eq!(c.anchor.fortress_id, FortressId::NIL);
        let mut game = self.game.borrow_mut();
        game.calls.push(Call::Observe);
        game.observe()
    }
    fn prepare(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        assert_eq!(c.anchor.fortress_id, FortressId::NIL);
        let mut game = self.game.borrow_mut();
        assert_eq!(plan.before(), &game.observe()?);
        game.calls.push(Call::Prepare);
        let record = native_record(plan, 0, 0, false, false, None)?;
        game.plans.insert(plan.key().to_owned(), plan.clone());
        game.records.insert(plan.key().to_owned(), record.clone());
        game.last = Some(plan.clone());
        Ok(record)
    }
    fn commit(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        // Decode actual coordinator bytes at the native boundary. This proves
        // dispatch custody exists before the mock unpause can happen.
        let mut journal = RunJournal::open(self.storage.clone(), c, RunMode::Offline, None, false)?;
        let entries = journal.records(c)?;
        let original = entries
            .iter()
            .find(|entry| entry.plan().key() == plan.key())
            .ok_or_else(exhausted)?;
        assert_eq!(original.state(), RunState::DispatchStarted);
        assert_eq!(original.plan(), plan);
        let mut game = self.game.borrow_mut();
        game.calls.push(Call::Commit);
        game.sequence += 1;
        game.paused = false;
        let record = native_record(plan, 1, 0, true, false, Some(game.tick))?;
        game.records.insert(plan.key().to_owned(), record.clone());
        if game.lose_commit {
            Err(error(
                ErrorCode::AdapterUnavailable,
                "lost clock commit reply",
            ))
        } else {
            Ok(record)
        }
    }
    fn query(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        assert_eq!(c.anchor.fortress_id, FortressId::NIL);
        let mut game = self.game.borrow_mut();
        game.calls.push(Call::Query);
        Ok(if game.absent {
            None
        } else {
            game.records.get(plan.key()).cloned()
        })
    }
    fn cancel(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        assert_eq!(c.anchor.fortress_id, FortressId::NIL);
        let mut game = self.game.borrow_mut();
        game.calls.push(Call::Cancel);
        game.paused = true;
        let record = native_record(plan, 3, 3, true, true, Some(game.tick))?;
        game.records.insert(plan.key().to_owned(), record.clone());
        Ok(record)
    }
}
struct Clocks {
    config: clock::Config,
    storage: Memory,
    game: Rc<RefCell<ClockGame>>,
    opens: usize,
    connects: usize,
    allow_control: bool,
    fail_after_commit: bool,
    fail_after_open: bool,
    opened_recovery: Option<ClockRecovery>,
    corrupt_after_dispatch: Option<Memory>,
    checkpoints: usize,
    corrupt_at_checkpoint: Option<usize>,
    corrupt_at_tick: Option<(u64, Memory)>,
}
impl clock::ClockIo<Memory> for Clocks {
    type Source = ClockNative;
    fn opening_recovery(&self) -> Option<ClockRecovery> {
        self.opened_recovery
    }
    fn open(
        &mut self,
        spec: &MonitorSpec,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockOwner<Memory>> {
        self.opens += 1;
        assert_eq!(n.anchor.fortress_id, FortressId::NIL);
        assert_ne!(c.anchor.fortress_id, FortressId::NIL);
        let journal = RunJournal::open(
            self.storage.clone(),
            n,
            RunMode::Control,
            Some(self.config.binding.clone()),
            self.config.initialize,
        )?;
        self.opened_recovery = Some(ClockRecovery {
            journal_id: journal.id(),
            head: journal.head(),
        });
        if self.fail_after_open {
            return Err(error(
                ErrorCode::CancellationRequested,
                "runtime rejected after clock header creation",
            ));
        }
        ClockOwner::new(journal, spec.clone(), self.config.recovery, c, n)
    }
    fn connect(&mut self, n: &OperationContext, control: bool) -> Result<ClockNative> {
        n.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if control {
            n.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            if !self.allow_control {
                return Err(error(ErrorCode::CapabilityDenied, "clock runtime disabled"));
            }
        }
        self.connects += 1;
        Ok(ClockNative {
            game: self.game.clone(),
            storage: self.storage.clone(),
            manifest: self.config.binding.manifest().clone(),
            endpoint: self.config.binding.endpoint(),
        })
    }
    fn checkpoint(&mut self, c: &OperationContext, control: bool) -> Result<()> {
        self.checkpoints += 1;
        if self.corrupt_at_checkpoint == Some(self.checkpoints) {
            self.storage.data.borrow_mut().get_mut()[0] ^= 1;
            self.corrupt_at_checkpoint = None;
        }
        if self
            .corrupt_at_tick
            .as_ref()
            .is_some_and(|(tick, _)| c.anchor.tick.get() >= *tick)
        {
            if let Some((_, storage)) = self.corrupt_at_tick.take() {
                storage.data.borrow_mut().get_mut()[0] ^= 1;
            }
        }
        c.budget.validate()?;
        if c.cancellation_requested
            || (self.fail_after_commit && self.game.borrow().count(Call::Commit) != 0)
        {
            return Err(error(
                ErrorCode::CancellationRequested,
                "clock runtime revoked",
            ));
        }
        if control && !self.allow_control {
            return Err(error(ErrorCode::CapabilityDenied, "clock runtime disabled"));
        }
        if control
            && last_state(&self.storage.snapshot_bytes()) == Some(RunState::DispatchStarted as u8)
        {
            if let Some(parent) = self.corrupt_after_dispatch.take() {
                parent.data.borrow_mut().get_mut()[0] ^= 1;
            }
        }
        Ok(())
    }
}
fn fixture() -> Result<(Harness, Maps, Clocks)> {
    let (mut h, mut maps) = completed(sparse_levels()?)?;
    success(&start(&mut h, &mut maps)?);
    let configured = clock::Config {
        path: "/private/dig/clock".into(),
        initialize: true,
        binding: RunBinding::new(
            h.config.endpoint,
            RunManifest {
                generation: 41,
                df_version: "test-df".into(),
                dfhack_version: "test-dfhack".into(),
            },
        )?,
        recovery: None,
    };
    h.config.clock = Some(configured.clone());
    h.state.clock_config = Some(configured.clone());
    h.c.budget.max_game_ticks = 1200;
    h.c.grants.push(CapabilityGrant {
        capability: Capability::ControlClock,
        scope: CapabilityScope {
            fortress_id: Some(h.c.anchor.fortress_id),
            ..CapabilityScope::default()
        },
        max_risk: RiskTier::Guarded,
        expires_at_tick: None,
        remaining_uses: None,
    });
    h.state.budget = h.c.budget;
    h.state.grants = h.c.grants.clone();
    h.state.clock_grants = runtime::clock_grants(true);
    Ok((
        h,
        maps,
        Clocks {
            config: configured,
            storage: Memory::default(),
            game: Rc::new(RefCell::new(ClockGame::default())),
            opens: 0,
            connects: 0,
            allow_control: true,
            fail_after_commit: false,
            fail_after_open: false,
            opened_recovery: None,
            corrupt_after_dispatch: None,
            checkpoints: 0,
            corrupt_at_checkpoint: None,
            corrupt_at_tick: None,
        },
    ))
}
fn dispatch(
    h: &mut Harness,
    maps: &mut Maps,
    clocks: &mut Clocks,
    op: &str,
    action: Action,
) -> Result<Value> {
    h.c.request_id = RequestId::new(h.c.request_id.get() + 1);
    h.c.anchor.tick = h.c.anchor.tick.max(GameTick(
        h.state.authority_tick.max(h.state.control.high_tick()),
    ));
    let game = h.game.clone();
    let raw = run_action_with_clock(
        &mut h.state,
        h.c.clone(),
        op,
        Ok(action),
        Instant::now(),
        &mut h.runtime,
        move |binding, region, _| {
            game.borrow_mut().factories += 1;
            Ok(Source {
                game,
                binding: binding.clone(),
                region,
                permit: None,
            })
        },
        maps,
        clocks,
    );
    assert!(
        raw.len() <= OUTPUT_BYTES as usize,
        "clock packet exceeded its output reservation"
    );
    serde_json::from_str(&raw).map_err(|_| exhausted())
}
fn clock_result(value: &Value) -> &Value {
    &value["result"]["blueprint_clock"]
}
fn goal_sample(
    h: &mut Harness,
    maps: &mut Maps,
    clocks: &mut Clocks,
    raw: LiveMapObservation,
) -> Result<Value> {
    let monitor_id = spec(h)?.id();
    maps.captures.push_back(raw);
    dispatch(
        h,
        maps,
        clocks,
        "fortress.wait",
        Action::SampleMonitor(monitor_id),
    )
}
fn request(h: &Harness, ticks: u32) -> Result<Action> {
    Ok(Action::PrepareClock(clock::Request {
        monitor_id: spec(h)?.id(),
        game_ticks: ticks,
        wall_millis: 1000,
    }))
}
fn ready(h: &Harness, tick: u64) -> Result<LiveMapObservation> {
    let mut raw = capture(h, tick)?;
    let mut tile = match visible(Shape::Wall) {
        Cell::Visible(tile) => tile,
        _ => return Err(exhausted()),
    };
    tile.dig_designation = 1;
    for point in spec(h)?.parent().layout().targets() {
        replace(&mut raw, *point, Cell::Visible(tile))?;
    }
    Ok(raw)
}
fn prepare(h: &mut Harness, maps: &mut Maps, clocks: &mut Clocks) -> Result<(Digest32, Value)> {
    maps.captures
        .push_back(ready(h, clocks.game.borrow().tick)?);
    let action = request(h, 10)?;
    let value = dispatch(h, maps, clocks, "fortress.plan", action)?;
    success(&value);
    let seal = digest(field(&clock_result(&value)["active_run"]["review_seal"])?)?;
    Ok((seal, value))
}
fn commit(h: &mut Harness, maps: &mut Maps, clocks: &mut Clocks, seal: Digest32) -> Result<Value> {
    maps.captures
        .push_back(ready(h, clocks.game.borrow().tick)?);
    let monitor_id = spec(h)?.id();
    dispatch(
        h,
        maps,
        clocks,
        "fortress.commit",
        Action::CommitClock { monitor_id, seal },
    )
}
fn wait(h: &mut Harness, maps: &mut Maps, clocks: &mut Clocks) -> Result<Value> {
    let id = spec(h)?.id();
    dispatch(h, maps, clocks, "fortress.wait", Action::WaitClock(id))
}
fn cancel(h: &mut Harness, maps: &mut Maps, clocks: &mut Clocks) -> Result<Value> {
    let id = spec(h)?.id();
    dispatch(h, maps, clocks, "fortress.cancel", Action::CancelClock(id))
}
fn inspect(
    h: &mut Harness,
    maps: &mut Maps,
    clocks: &mut Clocks,
    head: Option<Digest32>,
) -> Result<Value> {
    let id = spec(h)?.id();
    dispatch(
        h,
        maps,
        clocks,
        "fortress.query",
        Action::Query(Query::BlueprintClock {
            monitor_id: id.to_string(),
            expected_head: head.map(|v| v.to_string()),
            offset: None,
            limit: None,
        }),
    )
}
fn assert_no_completion(value: &Value) {
    assert_ne!(
        clock_result(value)["sampled_excavation_condition_proven"],
        true
    );
    assert_ne!(clock_result(value)["current_pause_proven"], true);
    assert_ne!(
        value["agent_turn"]["briefing"]["whole_blueprint_sampled_floor_condition_proven"],
        true
    );
}
fn reopen(h: &mut Harness, clocks: &mut Clocks, mode: RunMode) -> Result<ClockRecovery> {
    let owner = h.state.clock.take().ok_or_else(exhausted)?;
    let pin = owner.recovery();
    drop(owner);
    let c = h.c.clone();
    let n = clock::native_context(&c, &h.state.clock_grants)?;
    let journal = RunJournal::open(clocks.storage.clone(), &n, mode, None, false)?;
    let owner = ClockOwner::new(journal, spec(h)?.clone(), Some(pin), &c, &n)?;
    clocks.config.initialize = false;
    clocks.config.recovery = Some(pin);
    h.config.clock = Some(clocks.config.clone());
    h.state.clock_config = Some(clocks.config.clone());
    h.state.clock_recovery = Some(owner.recovery());
    h.state.authority_tick = h.state.authority_tick.max(owner.high_tick());
    h.state.clock = Some(owner);
    Ok(pin)
}

#[test]
fn one_reviewed_interval_advances_only_clock_and_requires_separate_whole_goal_samples() -> Result<()>
{
    let (mut h, mut maps, mut clocks) = fixture()?;
    let original = h.runtime.native.snapshot_bytes();
    let parent = h.runtime.parent.snapshot_bytes();
    let history = maps.history.snapshot_bytes();
    let native_calls = h.game.borrow().calls.len();
    let (seal, reviewed) = prepare(&mut h, &mut maps, &mut clocks)?;
    assert_eq!(clock_result(&reviewed)["readiness"]["pending_tiles"], 11);
    assert_eq!(clocks.game.borrow().count(Call::Prepare), 1);
    assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
    assert_eq!(maps.reads, 1);
    let committed = commit(&mut h, &mut maps, &mut clocks, seal)?;
    success(&committed);
    assert_eq!(clocks.game.borrow().count(Call::Commit), 1);
    assert_eq!(maps.reads, 2);
    assert_eq!(maps.history.snapshot_bytes(), history);
    assert_no_completion(&committed);
    let before_reads = maps.reads;
    let raw = capture(&h, TICK)?;
    let blocked = goal_sample(&mut h, &mut maps, &mut clocks, raw)?;
    assert_eq!(blocked["result"]["ok"], false);
    assert_eq!(maps.reads, before_reads);
    maps.captures.clear();
    clocks.game.borrow_mut().settle(TICK + 10)?;
    let settled = wait(&mut h, &mut maps, &mut clocks)?;
    success(&settled);
    assert_eq!(
        clock_result(&settled)["fresh_goal_sampling_permitted"],
        true
    );
    assert_eq!(clocks.game.borrow().count(Call::Query), 1);
    assert_eq!(clocks.game.borrow().count(Call::Commit), 1);
    assert_eq!(maps.reads, before_reads);
    assert_no_completion(&settled);
    for tick in [TICK + 10, TICK + 20] {
        let raw = capture(&h, tick)?;
        success(&goal_sample(&mut h, &mut maps, &mut clocks, raw)?);
    }
    assert_eq!(
        h.state
            .monitor
            .as_ref()
            .ok_or_else(exhausted)?
            .state()
            .status(),
        Status::Satisfied
    );
    assert_eq!(h.runtime.native.snapshot_bytes(), original);
    assert_eq!(h.runtime.parent.snapshot_bytes(), parent);
    assert_eq!(h.game.borrow().calls.len(), native_calls);
    Ok(())
}

#[test]
fn source_loss_and_missing_commit_reply_never_reenable_unpause_or_goal_sampling() -> Result<()> {
    for source_lost in [false, true] {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
        clocks.game.borrow_mut().lose_commit = true;
        let failed = commit(&mut h, &mut maps, &mut clocks, seal)?;
        assert_eq!(failed["result"]["ok"], false);
        assert_eq!(clocks.game.borrow().count(Call::Commit), 1);
        let pin = reopen(&mut h, &mut clocks, RunMode::Control)?;
        if source_lost {
            clocks.game.borrow_mut().lose_source()?;
        } else {
            clocks.game.borrow_mut().absent = true;
        }
        let queried = wait(&mut h, &mut maps, &mut clocks)?;
        assert_no_completion(&queried);
        let calls = clocks.game.borrow().calls.len();
        let repeated = commit(&mut h, &mut maps, &mut clocks, seal)?;
        assert_eq!(repeated["result"]["ok"], false);
        assert_eq!(clocks.game.borrow().count(Call::Commit), 1);
        let action = request(&h, 10)?;
        assert_eq!(
            dispatch(&mut h, &mut maps, &mut clocks, "fortress.plan", action)?["result"]["ok"],
            false
        );
        assert_eq!(clocks.game.borrow().count(Call::Prepare), 1);
        if source_lost {
            let stopped = cancel(&mut h, &mut maps, &mut clocks)?;
            assert_no_completion(&stopped);
            assert_eq!(clocks.game.borrow().calls.len(), calls);
        }
        let reads = maps.reads;
        let raw = capture(&h, TICK + 10)?;
        assert_eq!(
            goal_sample(&mut h, &mut maps, &mut clocks, raw)?["result"]["ok"],
            false
        );
        assert_eq!(maps.reads, reads);
        let report = inspect(&mut h, &mut maps, &mut clocks, None)?;
        success(&report);
        assert_eq!(
            clock_result(&report)["recovery"]["journal_id"],
            pin.journal_id.to_string()
        );
        assert_eq!(clock_result(&report)["unresolved"], true);
        assert_eq!(
            clock_result(&report)["fresh_goal_sampling_permitted"],
            false
        );
    }
    Ok(())
}

#[test]
fn post_dispatch_parent_tamper_prevents_native_commit_and_preserves_original_uncertainty()
-> Result<()> {
    let (mut h, mut maps, mut clocks) = fixture()?;
    let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
    let parent = h.runtime.parent.snapshot_bytes();
    clocks.corrupt_after_dispatch = Some(h.runtime.parent.clone());
    let failed = commit(&mut h, &mut maps, &mut clocks, seal)?;
    assert_eq!(failed["result"]["ok"], false);
    assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
    assert_eq!(
        last_state(&clocks.storage.snapshot_bytes()),
        Some(RunState::DispatchStarted as u8)
    );
    // Restoring the original fixture bytes cannot recreate the lost dispatch permit.
    *h.runtime.parent.data.borrow_mut().get_mut() = parent;
    let readback = wait(&mut h, &mut maps, &mut clocks)?;
    success(&readback);
    assert_eq!(
        clock_result(&readback)["active_run"]["native"]["phase"],
        "prepared"
    );
    assert_eq!(clock_result(&readback)["unresolved"], true);
    assert_eq!(
        commit(&mut h, &mut maps, &mut clocks, seal)?["result"]["ok"],
        false
    );
    assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
    Ok(())
}

#[test]
fn cancellation_needs_independent_clock_authority_but_no_future_goal_permission() -> Result<()> {
    let (mut h, mut maps, mut clocks) = fixture()?;
    let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
    success(&commit(&mut h, &mut maps, &mut clocks, seal)?);
    let reads = maps.reads;
    h.c.grants
        .retain(|g| matches!(g.capability, Capability::Query | Capability::ControlClock));
    h.state.grants = h.c.grants.clone();
    h.c.budget.max_game_ticks = 0;
    h.state.budget = h.c.budget;
    h.state
        .clock_grants
        .retain(|g| g.capability != Capability::Plan);
    let current = h.state.clock_grants.clone();
    h.state
        .clock_grants
        .retain(|g| g.capability != Capability::ControlClock);
    assert_eq!(
        cancel(&mut h, &mut maps, &mut clocks)?["result"]["ok"],
        false
    );
    assert_eq!(clocks.game.borrow().count(Call::Cancel), 0);
    h.state.clock_grants = current;
    let stopped = cancel(&mut h, &mut maps, &mut clocks)?;
    success(&stopped);
    assert_eq!(clocks.game.borrow().count(Call::Cancel), 1);
    assert_eq!(clocks.game.borrow().count(Call::Commit), 1);
    assert_eq!(maps.reads, reads);
    assert_eq!(
        clock_result(&stopped)["runs_page"]["runs"][0]["native"]["historical_pause_verified"],
        true
    );
    assert_no_completion(&stopped);
    Ok(())
}

#[test]
fn wrong_readiness_and_separate_authority_fail_before_native_prepare() -> Result<()> {
    for fault in 0..8 {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let mut raw = ready(&h, TICK)?;
        match fault {
            0 => h.state.clock_grants = h.c.grants.clone(),
            1 => {
                h.c.grants
                    .retain(|g| g.capability != Capability::ControlClock)
            }
            2 => h.c.budget.max_game_ticks = 0,
            3 => replace(&mut raw, [2, 2, 2], Cell::Hidden)?,
            4 => replace(&mut raw, [2, 2, 2], Cell::Unallocated)?,
            5 => replace(&mut raw, [2, 2, 2], wall())?,
            6 => {
                let Cell::Visible(mut tile) = visible(Shape::Floor) else {
                    return Err(exhausted());
                };
                tile.liquid_depth = 1;
                replace(&mut raw, [2, 2, 2], Cell::Visible(tile))?;
            }
            _ => raw.bridge_generation += 1,
        }
        maps.captures.push_back(raw);
        let action = request(&h, 10)?;
        let rejected = dispatch(&mut h, &mut maps, &mut clocks, "fortress.plan", action)?;
        assert_eq!(rejected["result"]["ok"], false, "fault {fault}: {rejected}");
        assert_eq!(clocks.game.borrow().count(Call::Prepare), 0);
        assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
        assert_eq!(
            h.state
                .monitor
                .as_ref()
                .ok_or_else(exhausted)?
                .state()
                .attempts(),
            0
        );
    }
    Ok(())
}

#[test]
fn restart_and_late_runtime_failure_preserve_exact_clock_recovery_without_a_review() -> Result<()> {
    let (mut h, mut maps, mut clocks) = fixture()?;
    let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
    let pin = reopen(&mut h, &mut clocks, RunMode::Control)?;
    assert_eq!(
        commit(&mut h, &mut maps, &mut clocks, seal)?["result"]["ok"],
        false
    );
    assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
    let retired = cancel(&mut h, &mut maps, &mut clocks)?;
    success(&retired);
    assert_eq!(clocks.game.borrow().count(Call::Cancel), 0);
    maps.captures.clear();
    // A cancelled immutable run key cannot be resurrected. A later external
    // paused clock observation creates a distinct interval for a new review.
    clocks.game.borrow_mut().tick = TICK + 1;
    let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
    clocks.fail_after_commit = true;
    let failed = commit(&mut h, &mut maps, &mut clocks, seal)?;
    assert_eq!(failed["result"]["ok"], false);
    assert_eq!(
        clock_result(&failed)["recovery"]["journal_id"],
        pin.journal_id.to_string()
    );
    assert_eq!(clock_result(&failed)["history_verified"], false);
    assert_eq!(
        clock_result(&failed)["active_run"]["review_seal"],
        Value::Null
    );
    assert_eq!(clocks.game.borrow().count(Call::Commit), 1);
    clocks.fail_after_commit = false;
    clocks.game.borrow_mut().settle(TICK + 11)?;
    let settled = wait(&mut h, &mut maps, &mut clocks)?;
    success(&settled);
    assert_no_completion(&settled);
    assert_eq!(clocks.game.borrow().count(Call::Commit), 1);
    Ok(())
}

#[test]
fn cleared_floor_stabilization_can_advance_but_fresh_satisfied_lookahead_cannot() -> Result<()> {
    for already_satisfied in [false, true] {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let raw = capture(&h, TICK)?;
        success(&goal_sample(&mut h, &mut maps, &mut clocks, raw)?);
        let history = maps.history.snapshot_bytes();
        clocks.game.borrow_mut().tick = TICK + if already_satisfied { 10 } else { 5 };
        maps.captures
            .push_back(capture(&h, clocks.game.borrow().tick)?);
        let action = request(&h, 10)?;
        let reviewed = dispatch(&mut h, &mut maps, &mut clocks, "fortress.plan", action)?;
        assert_eq!(maps.history.snapshot_bytes(), history);
        assert_eq!(
            h.state
                .monitor
                .as_ref()
                .ok_or_else(exhausted)?
                .state()
                .attempts(),
            1
        );
        assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
        if already_satisfied {
            assert_eq!(reviewed["result"]["ok"], false);
            assert_eq!(clocks.game.borrow().count(Call::Prepare), 0);
            assert_eq!(
                last_state(&clocks.storage.snapshot_bytes()),
                Some(RunState::Intent as u8)
            );
            success(&cancel(&mut h, &mut maps, &mut clocks)?);
            assert_eq!(clocks.game.borrow().count(Call::Cancel), 0);
        } else {
            success(&reviewed);
            assert_eq!(clock_result(&reviewed)["readiness"]["pending_tiles"], 0);
            assert_eq!(clock_result(&reviewed)["readiness"]["cleared_tiles"], 11);
            let seal = digest(field(
                &clock_result(&reviewed)["active_run"]["review_seal"],
            )?)?;
            maps.captures.push_back(capture(&h, TICK + 5)?);
            let id = spec(&h)?.id();
            success(&dispatch(
                &mut h,
                &mut maps,
                &mut clocks,
                "fortress.commit",
                Action::CommitClock {
                    monitor_id: id,
                    seal,
                },
            )?);
            assert_eq!(maps.history.snapshot_bytes(), history);
            clocks.game.borrow_mut().settle(TICK + 15)?;
            success(&wait(&mut h, &mut maps, &mut clocks)?);
        }
        let raw = capture(&h, clocks.game.borrow().tick)?;
        success(&goal_sample(&mut h, &mut maps, &mut clocks, raw)?);
        if already_satisfied {
            // A refused enclosing clock request conservatively withdraws the
            // old unfinished window. The next real sample starts again.
            assert_eq!(
                h.state
                    .monitor
                    .as_ref()
                    .ok_or_else(exhausted)?
                    .state()
                    .streak(),
                1
            );
            let raw = capture(&h, TICK + 20)?;
            success(&goal_sample(&mut h, &mut maps, &mut clocks, raw)?);
        }
        assert_eq!(
            h.state
                .monitor
                .as_ref()
                .ok_or_else(exhausted)?
                .state()
                .status(),
            Status::Satisfied
        );
        let calls = clocks.game.borrow().calls.len();
        let reads = maps.reads;
        let action = request(&h, 10)?;
        assert_eq!(
            dispatch(&mut h, &mut maps, &mut clocks, "fortress.plan", action)?["result"]["ok"],
            false
        );
        assert_eq!(clocks.game.borrow().calls.len(), calls);
        assert_eq!(maps.reads, reads);
    }
    Ok(())
}

#[test]
fn fresh_commit_readiness_change_is_retained_as_dispatched_uncertainty_without_unpause()
-> Result<()> {
    for changed in 0..3 {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
        let history = maps.history.snapshot_bytes();
        let mut raw = ready(&h, TICK)?;
        match changed {
            0 => replace(&mut raw, [2, 4, 3], Cell::Hidden)?,
            1 => raw.bridge_generation += 1,
            _ => raw.year_tick += 1,
        }
        maps.captures.push_back(raw);
        let id = spec(&h)?.id();
        let rejected = dispatch(
            &mut h,
            &mut maps,
            &mut clocks,
            "fortress.commit",
            Action::CommitClock {
                monitor_id: id,
                seal,
            },
        )?;
        assert_eq!(
            rejected["result"]["ok"], false,
            "readiness change {changed}: {rejected}"
        );
        assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
        assert_eq!(
            last_state(&clocks.storage.snapshot_bytes()),
            Some(RunState::DispatchStarted as u8)
        );
        assert_eq!(maps.history.snapshot_bytes(), history);
        assert_eq!(
            h.state
                .monitor
                .as_ref()
                .ok_or_else(exhausted)?
                .state()
                .attempts(),
            0
        );
        let readback = wait(&mut h, &mut maps, &mut clocks)?;
        success(&readback);
        assert_eq!(clock_result(&readback)["unresolved"], true);
        assert_eq!(
            clock_result(&readback)["fresh_goal_sampling_permitted"],
            false
        );
        assert_eq!(
            clock_result(&readback)["active_run"]["review_seal"],
            Value::Null
        );
        assert_no_completion(&readback);
    }
    Ok(())
}

#[test]
fn whole_action_budget_horizon_and_source_clock_agreement_precede_native_preparation() -> Result<()>
{
    for fault in 0..7 {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let mut ticks = 10;
        match fault {
            0 => h.c.budget.max_bytes = 32 * 1024 * 1024,
            1 => h.c.budget.max_entities = 1,
            2 => h.c.budget.max_output_tokens = 1,
            3 => {
                for grant in &mut h.c.grants {
                    if grant.capability == Capability::ControlClock {
                        grant.expires_at_tick = Some(GameTick(TICK + 5));
                    }
                }
            }
            4 => {
                for grant in &mut h.state.clock_grants {
                    if grant.capability == Capability::ControlClock {
                        grant.expires_at_tick = Some(GameTick(TICK + 5));
                    }
                }
            }
            5 => ticks = 1001,
            _ => clocks.game.borrow_mut().tick = TICK + 20,
        }
        maps.captures.push_back(ready(&h, TICK)?);
        let action = request(&h, ticks)?;
        let rejected = dispatch(&mut h, &mut maps, &mut clocks, "fortress.plan", action)?;
        assert_eq!(
            rejected["result"]["ok"], false,
            "preflight {fault}: {rejected}"
        );
        assert_eq!(clocks.game.borrow().count(Call::Prepare), 0);
        assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
        assert_eq!(
            h.state
                .monitor
                .as_ref()
                .ok_or_else(exhausted)?
                .state()
                .attempts(),
            0
        );
        if fault < 6 {
            assert_eq!(clocks.opens, 0);
            assert_eq!(clocks.connects, 0);
            assert_eq!(maps.reads, 0);
        }
    }
    Ok(())
}

#[test]
fn reconciled_overshoot_floor_rejects_older_goal_evidence_and_old_history_pages() -> Result<()> {
    let (mut h, mut maps, mut clocks) = fixture()?;
    let (seal, reviewed) = prepare(&mut h, &mut maps, &mut clocks)?;
    let old_head = digest(field(&clock_result(&reviewed)["recovery"]["head"])?)?;
    success(&commit(&mut h, &mut maps, &mut clocks, seal)?);
    clocks.game.borrow_mut().settle(TICK + 25)?;
    let settled = wait(&mut h, &mut maps, &mut clocks)?;
    success(&settled);
    assert_eq!(clock_result(&settled)["authority_tick_floor"], TICK + 25);
    assert_eq!(
        clock_result(&settled)["runs_page"]["runs"][0]["native"]["observed_tick_overshoot"],
        15
    );
    assert!(h.state.authority_tick >= TICK + 25);
    let bytes = clocks.storage.snapshot_bytes();
    let calls = clocks.game.borrow().calls.len();
    let stale = inspect(&mut h, &mut maps, &mut clocks, Some(old_head))?;
    assert_eq!(stale["result"]["ok"], false);
    success(&inspect(&mut h, &mut maps, &mut clocks, None)?);
    assert_eq!(clocks.game.borrow().calls.len(), calls);
    assert_eq!(clocks.storage.snapshot_bytes(), bytes);
    let old = capture(&h, TICK + 10)?;
    let sampled = goal_sample(&mut h, &mut maps, &mut clocks, old)?;
    // A completed read can successfully retain terminal negative evidence.
    // Its success envelope never accepts the older capture as goal progress.
    success(&sampled);
    assert_eq!(monitor_result(&sampled)["status"], "invalidated");
    assert_eq!(monitor_result(&sampled)["matching_samples"], 0);
    assert_eq!(monitor_result(&sampled)["fresh_sample_acquired"], false);
    let store = h.state.monitor.as_ref().ok_or_else(exhausted)?;
    assert!(store.high_tick() >= TICK + 25);
    assert_eq!(store.state().status(), Status::Invalidated);
    assert_no_completion(&sampled);
    reopen_history(&mut h, &mut maps, DigMode::Recover)?;
    let inspected = query(&mut h, &mut maps)?;
    success(&inspected);
    assert_eq!(monitor_result(&inspected)["status"], "invalidated");
    assert_eq!(
        monitor_result(&inspected)["authority_tick_floor"],
        TICK + 25
    );
    Ok(())
}

#[test]
fn offline_and_recovery_clock_owners_cannot_regain_a_commit_permit() -> Result<()> {
    for mode in [RunMode::Offline, RunMode::Recover] {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
        success(&commit(&mut h, &mut maps, &mut clocks, seal)?);
        let pin = reopen(&mut h, &mut clocks, mode)?;
        let commits = clocks.game.borrow().count(Call::Commit);
        assert_eq!(
            commit(&mut h, &mut maps, &mut clocks, seal)?["result"]["ok"],
            false
        );
        assert_eq!(clocks.game.borrow().count(Call::Commit), commits);
        assert_eq!(
            cancel(&mut h, &mut maps, &mut clocks)?["result"]["ok"],
            false
        );
        assert_eq!(clocks.game.borrow().count(Call::Cancel), 0);
        h.c.grants
            .retain(|grant| grant.capability == Capability::Query);
        h.state
            .clock_grants
            .retain(|grant| grant.capability == Capability::Query);
        h.c.budget.max_game_ticks = 0;
        let calls = clocks.game.borrow().calls.len();
        let bytes = clocks.storage.snapshot_bytes();
        let writes = *clocks.storage.writes.borrow();
        let syncs = *clocks.storage.syncs.borrow();
        let inspected = inspect(&mut h, &mut maps, &mut clocks, None)?;
        success(&inspected);
        assert_eq!(
            clock_result(&inspected)["recovery"]["journal_id"],
            pin.journal_id.to_string()
        );
        assert_eq!(clocks.game.borrow().calls.len(), calls);
        assert_eq!(clocks.storage.snapshot_bytes(), bytes);
        assert_eq!(*clocks.storage.writes.borrow(), writes);
        assert_eq!(*clocks.storage.syncs.borrow(), syncs);
        clocks.game.borrow_mut().settle(TICK + 10)?;
        let settled = wait(&mut h, &mut maps, &mut clocks)?;
        if mode == RunMode::Recover {
            success(&settled);
            assert_eq!(clocks.game.borrow().count(Call::Query), 1);
            assert_eq!(
                clock_result(&settled)["fresh_goal_sampling_permitted"],
                true
            );
        } else {
            assert_eq!(settled["result"]["ok"], false);
            assert_eq!(clocks.game.borrow().count(Call::Query), 0);
            assert_eq!(clocks.storage.snapshot_bytes(), bytes);
            assert_eq!(*clocks.storage.writes.borrow(), writes);
            assert_eq!(*clocks.storage.syncs.borrow(), syncs);
        }
        assert_eq!(clocks.game.borrow().count(Call::Commit), commits);
        assert_no_completion(&settled);
    }
    Ok(())
}

#[test]
fn standalone_monitor_recovery_has_no_clock_port_even_with_injected_grants() -> Result<()> {
    use crate::dig_control_server::monitor_recovery::RecoveryState;
    let (mut h, maps, clocks) = fixture()?;
    let monitor_id = spec(&h)?.id();
    let recovery = h.state.monitor.as_ref().ok_or_else(exhausted)?.recovery();
    h.config.blueprint = None;
    h.config.clock = None;
    let monitor = h.config.monitor.as_mut().ok_or_else(exhausted)?;
    monitor.initialize = false;
    monitor.recovery = Some(recovery);
    let mut c = h.c.clone();
    c.grants
        .retain(|grant| grant.capability == Capability::Query);
    let store = MonitorStore::open(
        maps.history.clone(),
        &c,
        DigMode::Recover,
        None,
        false,
        Some(recovery),
    )?;
    let mut isolated = RecoveryState::new(store, &c, &h.config, DigMode::Recover)?;
    let bytes = maps.history.snapshot_bytes();
    let calls = h.game.borrow().calls.len();
    let actions = [
        Action::PrepareClock(clock::Request {
            monitor_id,
            game_ticks: 10,
            wall_millis: 1000,
        }),
        Action::CommitClock {
            monitor_id,
            seal: Digest32::of_bytes(b"foreign-clock-review"),
        },
        Action::WaitClock(monitor_id),
        Action::CancelClock(monitor_id),
        Action::Query(Query::BlueprintClock {
            monitor_id: monitor_id.to_string(),
            expected_head: None,
            offset: None,
            limit: None,
        }),
    ];
    for action in actions {
        let mut current = isolated.context(None)?;
        current.grants = h.c.grants.clone();
        let raw = isolated.run_action(
            current,
            "fortress.query",
            Ok(action),
            Instant::now(),
            |_| Ok(()),
        );
        let value: Value = serde_json::from_str(&raw).map_err(|_| exhausted())?;
        assert_eq!(value["result"]["ok"], false);
        assert_eq!(value["result"]["monitor_only"], true);
        assert!(value["result"].get("blueprint_clock").is_none());
        assert_no_completion(&value);
    }
    assert_eq!(maps.history.snapshot_bytes(), bytes);
    assert_eq!(maps.reads, 0);
    assert_eq!(clocks.opens, 0);
    assert_eq!(clocks.connects, 0);
    assert_eq!(h.game.borrow().calls.len(), calls);
    Ok(())
}

#[test]
fn missing_owner_and_mislabeled_existing_clock_cannot_bypass_the_sampling_barrier() -> Result<()> {
    for initialize in [false, true] {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
        success(&commit(&mut h, &mut maps, &mut clocks, seal)?);
        let owner = h.state.clock.take().ok_or_else(exhausted)?;
        let recovery = owner.recovery();
        drop(owner);
        clocks.config.initialize = initialize;
        clocks.config.recovery = (!initialize).then_some(recovery);
        h.config.clock = Some(clocks.config.clone());
        h.state.clock_config = Some(clocks.config.clone());
        h.state.clock_recovery = (!initialize).then_some(recovery);
        let clock_bytes = clocks.storage.snapshot_bytes();
        let monitor_bytes = maps.history.snapshot_bytes();
        let calls = clocks.game.borrow().calls.len();
        let reads = maps.reads;
        let raw = capture(&h, TICK + 20)?;
        let rejected = goal_sample(&mut h, &mut maps, &mut clocks, raw)?;
        assert_eq!(rejected["result"]["ok"], false);
        assert_eq!(maps.reads, reads);
        assert_eq!(clocks.game.borrow().calls.len(), calls);
        assert_eq!(clocks.storage.snapshot_bytes(), clock_bytes);
        assert_eq!(maps.history.snapshot_bytes(), monitor_bytes);
        assert_eq!(clocks.game.borrow().count(Call::Commit), 1);
    }
    Ok(())
}

#[test]
fn required_checkpoint_refuses_new_clock_work_without_blocking_local_retirement() -> Result<()> {
    for prepared in [false, true] {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let seal = if prepared {
            Some(prepare(&mut h, &mut maps, &mut clocks)?.0)
        } else {
            None
        };
        h.state.policy = DigControlPolicy::new(
            h.state.binding.clone(),
            h.state.policy.journal_id(),
            h.state.id,
            h.state.policy.lease_id(),
            h.state.policy.protected_areas().to_vec(),
            DigCheckpointPolicy::Required,
        )?;
        let calls = clocks.game.borrow().calls.len();
        let reads = maps.reads;
        let bytes = clocks.storage.snapshot_bytes();
        let rejected = if let Some(seal) = seal {
            commit(&mut h, &mut maps, &mut clocks, seal)?
        } else {
            let action = request(&h, 10)?;
            dispatch(&mut h, &mut maps, &mut clocks, "fortress.plan", action)?
        };
        assert_eq!(rejected["result"]["ok"], false);
        assert_eq!(clocks.game.borrow().calls.len(), calls);
        assert_eq!(maps.reads, reads);
        assert_eq!(clocks.storage.snapshot_bytes(), bytes);
        if prepared {
            success(&cancel(&mut h, &mut maps, &mut clocks)?);
            assert_eq!(clocks.game.borrow().count(Call::Cancel), 0);
            assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
            success(&inspect(&mut h, &mut maps, &mut clocks, None)?);
        }
    }
    Ok(())
}

#[test]
fn negative_readiness_withdraws_the_original_window_without_counting_goal_samples() -> Result<()> {
    for reopen_monitor in [false, true] {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let raw = capture(&h, TICK)?;
        success(&goal_sample(&mut h, &mut maps, &mut clocks, raw)?);
        let history = maps.history.snapshot_bytes();
        let original_id = spec(&h)?.id();
        let original_deadline = spec(&h)?.policy().deadline_tick();
        clocks.game.borrow_mut().tick = TICK + 5;
        let (_, reviewed) = prepare(&mut h, &mut maps, &mut clocks)?;
        assert_eq!(clock_result(&reviewed)["readiness"]["pending_tiles"], 11);
        assert_eq!(maps.history.snapshot_bytes(), history);
        let store = h.state.monitor.as_ref().ok_or_else(exhausted)?;
        assert_eq!(store.state().attempts(), 1);
        assert_eq!(store.state().streak(), 0);
        success(&cancel(&mut h, &mut maps, &mut clocks)?);
        assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
        if reopen_monitor {
            reopen_history(&mut h, &mut maps, DigMode::Control)?;
        }
        let raw = capture(&h, TICK + 15)?;
        success(&goal_sample(&mut h, &mut maps, &mut clocks, raw)?);
        let store = h.state.monitor.as_ref().ok_or_else(exhausted)?;
        assert_eq!(store.state().attempts(), 2);
        assert_eq!(store.state().streak(), 1);
        assert_ne!(store.state().status(), Status::Satisfied);
        assert_ne!(maps.history.snapshot_bytes(), history);
        assert_eq!(spec(&h)?.id(), original_id);
        assert_eq!(spec(&h)?.policy().deadline_tick(), original_deadline);
        let raw = capture(&h, TICK + 25)?;
        success(&goal_sample(&mut h, &mut maps, &mut clocks, raw)?);
        assert_eq!(
            h.state
                .monitor
                .as_ref()
                .ok_or_else(exhausted)?
                .state()
                .status(),
            Status::Satisfied
        );
    }
    Ok(())
}

#[test]
fn complete_clock_header_survives_failed_open_publication_and_recovers_exactly() -> Result<()> {
    for sampling in [false, true] {
        let (mut h, mut maps, mut clocks) = fixture()?;
        clocks.fail_after_open = true;
        let rejected = if sampling {
            let raw = capture(&h, TICK)?;
            goal_sample(&mut h, &mut maps, &mut clocks, raw)?
        } else {
            let action = request(&h, 10)?;
            dispatch(&mut h, &mut maps, &mut clocks, "fortress.plan", action)?
        };
        assert_eq!(rejected["result"]["ok"], false);
        let pin = clocks.opened_recovery.ok_or_else(exhausted)?;
        assert_eq!(clock_result(&rejected)["recovery"], clock::recovery(pin));
        assert_eq!(clock_result(&rejected)["history_verified"], false);
        assert_eq!(h.state.clock_recovery, Some(pin));
        assert!(h.state.clock.is_none());
        assert_eq!(clocks.opens, 1);
        assert_eq!(clocks.connects, 0);
        assert_eq!(maps.reads, 0);
        let n = clock::native_context(&h.c, &h.state.clock_grants)?;
        let mut journal =
            RunJournal::open(clocks.storage.clone(), &n, RunMode::Offline, None, false)?;
        assert_eq!(journal.id(), pin.journal_id);
        assert_eq!(journal.head(), pin.head);
        assert!(journal.records(&n)?.is_empty());
        drop(journal);
        clocks.fail_after_open = false;
        clocks.config.initialize = false;
        clocks.config.recovery = Some(pin);
        h.config.clock = Some(clocks.config.clone());
        h.state.clock_config = Some(clocks.config.clone());
        maps.captures.clear();
        let raw = capture(&h, TICK)?;
        success(&goal_sample(&mut h, &mut maps, &mut clocks, raw)?);
        assert_eq!(clocks.opens, 2);
        assert_eq!(clocks.connects, 0);
        assert_eq!(clocks.game.borrow().calls.len(), 0);
        assert_eq!(
            h.state.clock.as_ref().ok_or_else(exhausted)?.recovery(),
            pin
        );
        assert_eq!(
            h.state
                .monitor
                .as_ref()
                .ok_or_else(exhausted)?
                .state()
                .streak(),
            1
        );
    }
    Ok(())
}

#[test]
fn final_runtime_boundary_cannot_publish_clock_history_it_just_invalidated() -> Result<()> {
    for operation in ["prepare", "query", "commit"] {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let rejected = if operation == "prepare" {
            maps.captures.push_back(ready(&h, TICK)?);
            // Initial runtime check, two guard checks, post-operation check,
            // then the final publication check after the packet is rendered.
            clocks.corrupt_at_checkpoint = Some(5);
            let action = request(&h, 10)?;
            dispatch(&mut h, &mut maps, &mut clocks, "fortress.plan", action)?
        } else {
            let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
            let final_check = if operation == "query" { 3 } else { 5 };
            clocks.corrupt_at_checkpoint = Some(clocks.checkpoints + final_check);
            if operation == "query" {
                inspect(&mut h, &mut maps, &mut clocks, None)?
            } else {
                commit(&mut h, &mut maps, &mut clocks, seal)?
            }
        };
        assert_eq!(rejected["result"]["ok"], false, "{operation}: {rejected}");
        assert_eq!(clock_result(&rejected)["history_verified"], false);
        assert_eq!(
            clock_result(&rejected)["active_run"]["review_seal"],
            Value::Null
        );
        assert!(
            h.state
                .clock
                .as_ref()
                .ok_or_else(exhausted)?
                .prepared_seal()
                .is_none()
        );
        assert_eq!(
            rejected["result"]["effect_may_have_occurred"],
            operation == "commit"
        );
        assert_eq!(
            clocks.game.borrow().count(Call::Commit),
            usize::from(operation == "commit")
        );
        assert!(h.state.clock_recovery.is_some());
        assert_no_completion(&rejected);
    }
    Ok(())
}

#[test]
fn rejected_post_capture_custody_preserves_consumed_clock_floor_in_handoff() -> Result<()> {
    let (mut h, mut maps, mut clocks) = fixture()?;
    let learned = TICK + 40;
    maps.captures.push_back(ready(&h, learned)?);
    clocks.corrupt_at_tick = Some((learned, h.runtime.parent.clone()));
    let action = request(&h, 10)?;
    let rejected = dispatch(&mut h, &mut maps, &mut clocks, "fortress.plan", action)?;
    assert_eq!(rejected["result"]["ok"], false);
    assert!(h.state.authority_tick >= learned);
    assert!(
        clock_result(&rejected)["authority_tick_floor"]
            .as_u64()
            .is_some_and(|tick| tick >= learned)
    );
    assert_eq!(
        clock_result(&rejected)["active_run"]["review_seal"],
        Value::Null
    );
    // The blueprint owner captures its run witness, then the durable run
    // coordinator independently rechecks that witness before the map guard.
    assert_eq!(clocks.game.borrow().count(Call::Observe), 2);
    assert_eq!(clocks.game.borrow().count(Call::Prepare), 0);
    assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
    assert_eq!(maps.reads, 1);
    assert_no_completion(&rejected);
    Ok(())
}

#[test]
fn outer_worker_acknowledgement_controls_review_reuse_without_clock_replay() -> Result<()> {
    for acknowledged in [false, true] {
        let (mut h, mut maps, mut clocks) = fixture()?;
        let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
        let pin = h.state.clock_recovery.ok_or_else(exhausted)?;
        let floor = h.state.authority_tick;
        h.state.worker_acknowledgement = Some(Arc::new(AtomicBool::new(acknowledged)));
        h.c = h.state.context_with_clock(true, true, None)?;
        let connects = clocks.connects;
        let outcome = commit(&mut h, &mut maps, &mut clocks, seal)?;
        if acknowledged {
            success(&outcome);
            assert_eq!(clocks.game.borrow().count(Call::Commit), 1);
        } else {
            assert_eq!(outcome["result"]["ok"], false);
            assert_eq!(clocks.connects, connects);
            assert_eq!(clocks.game.borrow().count(Call::Commit), 0);
            assert_eq!(h.state.clock_recovery, Some(pin));
            assert!(h.state.authority_tick >= floor);
        }
    }
    Ok(())
}

#[path = "clock_tests/recovery_tests.rs"]
mod recovery_tests;
