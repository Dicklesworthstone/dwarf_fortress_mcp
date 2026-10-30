//! Complete original-room clock cycles use real private room, placement,
//! parent and run journals. Native calls are counted injected protocol records.
use super::super::super::room_clock;
use super::*;
use dfmcp_adapter::room_condition::SourceIdentity;
use dfmcp_adapter::room_condition::clock::Clock as RoomClock;

#[path = "room_clock_cross_tests.rs"]
mod room_clock_cross_tests;

pub(super) fn opening_pin(config: &room_clock::Config, n: &OperationContext) -> Recovery {
    config.recovery.unwrap_or_else(|| {
        let id = config
            .binding
            .initial_journal_id(n.session_id, n.request_id);
        Recovery {
            journal_id: id,
            head: id,
        }
    })
}

pub(super) fn open_owner(
    config: &room_clock::Config,
    definition: &Definition,
    source: &SourceIdentity,
    c: &OperationContext,
    n: &OperationContext,
) -> Result<RoomClock<PrivateRunFile>> {
    let journal = open_private_run_journal_exact(
        &config.path,
        n,
        RunMode::Control,
        Some(config.binding.clone()),
        config.initialize,
    )?;
    RoomClock::new(
        journal,
        definition.clone(),
        source.clone(),
        config.recovery,
        c,
        n,
    )
}

struct RoomClockIo {
    game: Rc<RefCell<RunGame>>,
    manifest: RunManifest,
    samples: VecDeque<Sample>,
    complete: bool,
    terrain: TerrainFault,
    acquisitions: usize,
    connections: usize,
    checkpoints: usize,
    deny_control: bool,
    lose_parent_after_sample: Option<PathBuf>,
    opening: Option<Recovery>,
    floor: u64,
    fail_sample: bool,
    fail_open: bool,
    checkpoint_fault: Option<CheckpointFault>,
    fault_on_checkpoint: Option<usize>,
}
impl room_clock::Io for RoomClockIo {
    type Source = RunNative;
    fn opening_recovery(&self) -> Option<Recovery> {
        self.opening
    }
    fn open(
        &mut self,
        config: &room_clock::Config,
        definition: &Definition,
        source: &SourceIdentity,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<RoomClock<PrivateRunFile>> {
        self.opening = Some(opening_pin(config, n));
        if self.fail_open {
            return Err(error(
                ErrorCode::CorruptLedger,
                "injected clock initialization failure",
            ));
        }
        let owner = open_owner(config, definition, source, c, n)?;
        self.opening = Some(owner.recovery());
        Ok(owner)
    }
    fn connect(
        &mut self,
        config: &room_clock::Config,
        _: &OperationContext,
        control: bool,
    ) -> Result<Self::Source> {
        if control && self.deny_control {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "room clock opt-in withdrawn",
            ));
        }
        assert_eq!(config.binding.manifest(), &self.manifest);
        self.connections += 1;
        Ok(RunNative {
            manifest: self.manifest.clone(),
            game: self.game.clone(),
        })
    }
    fn sample(&mut self, definition: &Definition, _: &OperationContext) -> Result<Sample> {
        self.acquisitions += 1;
        let tick = self.game.borrow().tick;
        self.floor = self.floor.max(tick);
        if std::mem::take(&mut self.fail_sample) {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "room bracket lost after consumed map clock",
            ));
        }
        let sample = match self.samples.pop_front() {
            Some(sample) => sample,
            None => room_sample(definition, tick, self.complete, self.terrain)?,
        };
        if let Some(path) = self.lose_parent_after_sample.take() {
            fs::remove_file(path).map_err(storage_error)?;
        }
        Ok(sample)
    }
    fn observed_tick_floor(&self) -> u64 {
        self.floor
    }
    fn checkpoint(&mut self, _: &OperationContext, control: bool) -> Result<()> {
        self.checkpoints += 1;
        if control && self.deny_control {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "room clock authority revoked",
            ));
        }
        if self.fault_on_checkpoint == Some(self.checkpoints) {
            match self.checkpoint_fault.take() {
                Some(CheckpointFault::Reject) => {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "room clock final checkpoint revoked",
                    ));
                }
                Some(CheckpointFault::Remove(path)) => {
                    fs::remove_file(path).map_err(storage_error)?
                }
                Some(CheckpointFault::Corrupt(path)) => {
                    let mut raw = fs::read(&path).map_err(storage_error)?;
                    *raw.last_mut().ok_or_else(exhausted)? ^= 1;
                    fs::write(path, raw).map_err(storage_error)?;
                }
                None => {}
            }
        }
        Ok(())
    }
}

struct RoomRunHarness {
    room: RoomHarness,
    io: RoomClockIo,
}
impl RoomRunHarness {
    fn bootstrap(count: usize, max_observations: u32, complete: bool) -> Result<Self> {
        let mut room = setup_room(count)?;
        room.state.budget.max_game_ticks = 1200;
        room.state.grants.push(CapabilityGrant {
            capability: Capability::ControlClock,
            scope: CapabilityScope {
                fortress_id: Some(room.state.binding.fortress().fortress_id()),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        });
        let manifest = RunManifest {
            generation: 41,
            df_version: "test-df".into(),
            dfhack_version: "test-dfhack".into(),
        };
        let selected = room_clock::Config {
            path: room.directory.file("room-clock"),
            initialize: true,
            binding: RunBinding::new(room.state.binding.endpoint(), manifest.clone())?,
            recovery: None,
        };
        room.config.room_clock = Some(selected.clone());
        room.state.policy.config.room_clock = Some(selected.clone());
        room.state.room_clock_config = Some(selected);
        room.state.room_clock_grants = clock_grants();
        room.io.complete = complete;
        let mut timing = room_timing();
        timing.max_observations = max_observations;
        let started = room.call(room_monitor::Action::Start(timing))?;
        assert_eq!(started["result"]["ok"], true, "{started}");
        assert!(room.state.room_clock.is_none());
        let sampled = room.call(room_monitor::Action::Sample)?;
        assert_eq!(sampled["result"]["ok"], true, "{sampled}");
        assert!(
            room.state.room_clock.is_none(),
            "a Query-only bootstrap must not capture permanent Plan authority"
        );
        assert!(
            !room
                .state
                .room_clock_config
                .as_ref()
                .ok_or_else(missing_fixture)?
                .path
                .exists()
        );
        assert!(room.state.clock.is_none());
        assert!(room.state.completion.is_none());
        let mut game = RunGame::new();
        game.tick = room.io.tick;
        let io = RoomClockIo {
            game: Rc::new(RefCell::new(game)),
            manifest,
            samples: VecDeque::new(),
            complete,
            terrain: TerrainFault::None,
            acquisitions: 0,
            connections: 0,
            checkpoints: 0,
            deny_control: false,
            lose_parent_after_sample: None,
            opening: None,
            floor: 0,
            fail_sample: false,
            fail_open: false,
            checkpoint_fault: None,
            fault_on_checkpoint: None,
        };
        Ok(Self { room, io })
    }
    fn setup(count: usize, max_observations: u32, complete: bool) -> Result<Self> {
        let mut h = Self::bootstrap(count, max_observations, complete)?;
        // This is the same opening helper used by explicit room-clock plan.
        let c = h.room.state.context(true, None)?;
        room_clock::open(
            &mut h.room.state,
            &c,
            &mut Work::new(&c, Instant::now())?,
            &mut h.io,
        )?;
        assert!(h.room.state.room_clock.is_some());
        assert_eq!(h.io.connections, 0);
        Ok(h)
    }
    fn id(&self) -> Result<Digest32> {
        Ok(self.room.definition()?.id())
    }
    fn clock(&self) -> Result<&RoomClock<PrivateRunFile>> {
        self.room
            .state
            .room_clock
            .as_ref()
            .ok_or_else(missing_fixture)
    }
    fn call(&mut self, action: room_clock::Action) -> Result<Value> {
        let op = match action {
            room_clock::Action::Prepare(_) => "fortress.plan",
            room_clock::Action::Commit { .. } => "fortress.commit",
            room_clock::Action::Wait(_) => "fortress.wait",
            room_clock::Action::Cancel(_) => "fortress.cancel",
            room_clock::Action::Inspect { .. } => "fortress.query",
        };
        let c = self.room.state.context(true, None)?;
        let raw = room_clock::run_action(
            &mut self.room.state,
            c,
            op,
            action,
            Instant::now(),
            &mut self.io,
        );
        assert!(raw.len() as u64 <= OUTPUT_BYTES);
        serde_json::from_str(&raw).map_err(|_| exhausted())
    }
    fn prepare(&mut self) -> Result<Value> {
        self.call(room_clock::Action::Prepare(room_clock::Request {
            monitor_id: self.id()?,
            game_ticks: 10,
            wall_millis: 5000,
        }))
    }
    fn seal(&self) -> Result<Digest32> {
        self.clock()?.prepared_seal().ok_or_else(missing_fixture)
    }
    fn commit(&mut self) -> Result<Value> {
        self.call(room_clock::Action::Commit {
            id: self.id()?,
            seal: self.seal()?,
        })
    }
    fn settle(&mut self) -> Result<Value> {
        self.io.game.borrow_mut().finish()?;
        self.call(room_clock::Action::Wait(self.id()?))
    }
    fn sample(&mut self) -> Result<Value> {
        self.room.io.tick = self.io.game.borrow().tick;
        self.room.io.complete = self.io.complete;
        self.room.io.terrain = self.io.terrain;
        self.room.call(room_monitor::Action::Sample)
    }
    fn reopened(&mut self) -> Result<()> {
        let pin = self.clock()?.recovery();
        self.room.state.room_clock = None;
        let mut selected = self
            .room
            .state
            .room_clock_config
            .clone()
            .ok_or_else(missing_fixture)?;
        selected.initialize = false;
        selected.recovery = Some(pin);
        self.room.state.room_clock_recovery = Some(pin);
        self.room.state.room_clock_config = Some(selected.clone());
        self.room.config.room_clock = Some(selected.clone());
        self.room.state.policy.config.room_clock = Some(selected);
        let c = self.room.state.context(false, None)?;
        room_clock::open(
            &mut self.room.state,
            &c,
            &mut Work::new(&c, Instant::now())?,
            &mut self.io,
        )
    }
}

#[test]
fn complete_original_room_runs_and_resamples_without_positive_readiness_credit() -> Result<()> {
    for count in [1, 32] {
        let mut h = RoomRunHarness::setup(count, 32, true)?;
        let original = h.id()?;
        assert_ne!(original, h.room.definition()?.original().id());
        assert_eq!(
            (h.room.progress()?.observations, h.room.progress()?.streak),
            (1, 1)
        );
        let prepared = h.prepare()?;
        assert_eq!(prepared["result"]["ok"], true, "{prepared}");
        assert_eq!(prepared["result"]["confirmation_required"], true);
        assert_eq!(prepared["result"]["goal_sample_published"], false);
        assert_eq!(h.room.progress()?.observations, 2);
        assert_eq!(h.room.progress()?.streak, 1);
        assert!(!h.room.progress()?.terminal());
        assert_eq!(h.room.progress()?.assessments.len(), count);
        assert!(h.clock()?.cached_view().pending);
        assert_eq!(h.clock()?.definition().id(), original);
        let calls = h.io.game.borrow().calls.clone();
        assert!(!calls.contains(&"commit"));
        let committed = h.commit()?;
        assert_eq!(committed["result"]["ok"], true, "{committed}");
        assert_eq!(
            (h.room.progress()?.observations, h.room.progress()?.streak),
            (3, 1)
        );
        assert_eq!(h.io.acquisitions, 2);
        let blocked = h.sample()?;
        assert_eq!(blocked["result"]["ok"], false, "{blocked}");
        assert_eq!(h.room.io.samples, 1);
        let settled = h.settle()?;
        assert_eq!(settled["result"]["ok"], true, "{settled}");
        assert!(!h.clock()?.cached_view().pending);
        let sampled = h.sample()?;
        assert_eq!(sampled["result"]["ok"], true, "{sampled}");
        // The refused intervening call deliberately withdrew continuity.
        assert!(!h.room.progress()?.terminal());
        assert_eq!(h.room.progress()?.streak, 1);
        let second = h.prepare()?;
        assert_eq!(second["result"]["ok"], true, "{second}");
        assert_eq!(h.commit()?["result"]["ok"], true);
        assert_eq!(h.settle()?["result"]["ok"], true);
        let complete = h.sample()?;
        assert_eq!(complete["result"]["ok"], true, "{complete}");
        assert_eq!(h.room.progress()?.phase, "satisfied");
        assert_eq!(h.room.progress()?.streak, 2);
        assert_eq!(h.room.progress()?.condition_met_count(), count);
        assert_eq!(h.room.progress()?.terrain.furnishings.matched, count as u32);
        assert_eq!(h.id()?, original);
        let head = h.clock()?.recovery().head;
        let page = h.call(room_clock::Action::Inspect {
            id: original,
            head,
            offset: 0,
            limit: 1,
        })?;
        assert_eq!(page["result"]["ok"], true, "{page}");
        let page = &page["result"]["room_clock"]["runs_page"];
        assert_eq!(page["total"], 2);
        assert_eq!(
            page["runs"].as_array().ok_or_else(missing_fixture)?.len(),
            1
        );
        assert_eq!(page["next_query"]["mode"], "room_clock");
        assert_eq!(page["next_query"]["limit"], 1);
        let last = h.call(room_clock::Action::Inspect {
            id: original,
            head,
            offset: 1,
            limit: 1,
        })?;
        assert_eq!(last["result"]["ok"], true, "{last}");
        let last = &last["result"]["room_clock"]["runs_page"];
        assert_eq!(last["offset"], 1);
        assert_eq!(
            last["runs"].as_array().ok_or_else(missing_fixture)?.len(),
            1
        );
        assert!(last["next_query"].is_null());
        assert_eq!(
            h.io.game
                .borrow()
                .calls
                .iter()
                .filter(|&&v| v == "commit")
                .count(),
            2
        );
        assert_eq!(h.prepare()?["result"]["ok"], false);
    }
    Ok(())
}

#[test]
fn known_room_terrain_deficits_remain_pending_when_all_furniture_is_complete() -> Result<()> {
    let mut h = RoomRunHarness::setup(2, 32, true)?;
    h.io.terrain = TerrainFault::WallMissing;
    let prepared = h.prepare()?;
    assert_eq!(prepared["result"]["ok"], true, "{prepared}");
    assert_eq!(h.room.progress()?.condition_met_count(), 2);
    assert!(h.room.progress()?.terrain.walls.wrong_shape > 0);
    assert_eq!(h.room.progress()?.streak, 0);
    assert_eq!(h.room.progress()?.phase, "active");
    let ready = h.clock()?.cached_view().readiness;
    assert!(ready.is_none(), "cached views never grant live readiness");
    assert_eq!(h.commit()?["result"]["ok"], true);
    assert_eq!(h.settle()?["result"]["ok"], true);
    assert_eq!(h.sample()?["result"]["ok"], true);
    assert!(!h.room.progress()?.terminal());
    assert_eq!(h.room.progress()?.streak, 0);
    assert!(!h.room.progress()?.terrain.condition_met());
    Ok(())
}

#[test]
fn unknown_room_terrain_refuses_clock_and_persists_negative_joint_readiness() -> Result<()> {
    let mut h = RoomRunHarness::setup(2, 32, true)?;
    h.io.terrain = TerrainFault::FloorHidden;
    let denied = h.prepare()?;
    assert_eq!(denied["result"]["ok"], false, "{denied}");
    assert_eq!(h.io.acquisitions, 1);
    assert_eq!(h.room.progress()?.observations, 2);
    assert_eq!(h.room.progress()?.streak, 0);
    assert!(h.room.progress()?.terrain.floors.hidden > 0);
    assert!(!h.io.game.borrow().calls.contains(&"prepare"));
    assert!(!h.io.game.borrow().calls.contains(&"commit"));
    assert!(h.clock()?.prepared_seal().is_none());
    let before = h.room.monitor()?.store.recovery();
    h.room.reopen()?;
    assert_eq!(h.room.monitor()?.store.recovery(), before);
    assert_eq!(h.room.progress()?.streak, 0);
    assert!(h.room.progress()?.terrain.floors.hidden > 0);
    Ok(())
}

#[test]
fn original_room_acquisition_limit_expires_before_prepare_or_commit_unpause() -> Result<()> {
    for allowance in [2, 3] {
        let mut h = RoomRunHarness::setup(2, allowance, false)?;
        let prepared = h.prepare()?;
        if allowance == 2 {
            assert_eq!(prepared["result"]["ok"], false, "{prepared}");
            assert!(!h.io.game.borrow().calls.contains(&"prepare"));
        } else {
            assert_eq!(prepared["result"]["ok"], true, "{prepared}");
            let denied = h.commit()?;
            assert_eq!(denied["result"]["ok"], false, "{denied}");
        }
        assert_eq!(h.room.progress()?.observations, allowance);
        assert_eq!(h.room.progress()?.phase, "expired");
        assert_eq!(h.room.progress()?.streak, 0);
        assert!(h.clock()?.prepared_seal().is_none());
        assert!(!h.io.game.borrow().calls.contains(&"commit"));
        let original_limit = h.room.definition()?.goal().timing().max_observations;
        assert_eq!(original_limit, allowance);
    }
    Ok(())
}

#[test]
fn lost_room_clock_commit_reply_requires_original_reconciliation_without_duplicate_dispatch()
-> Result<()> {
    let mut h = RoomRunHarness::setup(2, 32, false)?;
    assert_eq!(h.prepare()?["result"]["ok"], true);
    let original = h.clock()?.cached_view().runs[0].entry.plan().clone();
    h.io.game.borrow_mut().lose_commit = true;
    let lost = h.commit()?;
    assert_eq!(lost["result"]["ok"], false, "{lost}");
    assert!(h.clock()?.cached_view().unresolved);
    assert!(h.clock()?.prepared_seal().is_none());
    let acquisitions = h.io.acquisitions;
    assert_eq!(h.prepare()?["result"]["ok"], false);
    assert_eq!(h.io.acquisitions, acquisitions);
    assert_eq!(h.sample()?["result"]["ok"], false);
    h.io.game.borrow_mut().absent = true;
    let unknown = h.call(room_clock::Action::Wait(h.id()?))?;
    assert!(h.clock()?.cached_view().pending, "{unknown}");
    assert!(h.clock()?.cached_view().unresolved);
    assert_eq!(h.prepare()?["result"]["ok"], false);
    h.io.game.borrow_mut().absent = false;
    let settled = h.settle()?;
    assert_eq!(settled["result"]["ok"], true, "{settled}");
    assert!(!h.clock()?.cached_view().pending);
    assert_eq!(h.clock()?.cached_view().runs[0].entry.plan(), &original);
    assert_eq!(
        h.io.game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "commit")
            .count(),
        1
    );
    h.io.complete = true;
    let sampled = h.sample()?;
    assert_eq!(sampled["result"]["ok"], true, "{sampled}");
    assert_eq!(h.room.progress()?.streak, 1);
    assert!(!h.room.progress()?.terminal());
    Ok(())
}

#[test]
fn reopened_room_clock_keeps_original_history_and_never_restores_a_review_seal() -> Result<()> {
    let mut h = RoomRunHarness::setup(2, 32, false)?;
    assert_eq!(h.prepare()?["result"]["ok"], true);
    let seal = h.seal()?;
    let pin = h.clock()?.recovery();
    let goal = h.id()?;
    let timing = h.room.definition()?.goal().timing();
    let source = h.clock()?.source().clone();
    h.reopened()?;
    assert_eq!(h.clock()?.recovery(), pin);
    assert_eq!(h.clock()?.source(), &source);
    assert_eq!(h.clock()?.definition().id(), goal);
    assert_eq!(h.clock()?.definition().goal().timing(), timing);
    assert!(h.clock()?.prepared_seal().is_none());
    let refused = h.call(room_clock::Action::Commit { id: goal, seal })?;
    assert_eq!(refused["result"]["ok"], false, "{refused}");
    assert!(!h.io.game.borrow().calls.contains(&"commit"));
    let cancelled = h.call(room_clock::Action::Cancel(goal))?;
    assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
    assert!(!h.clock()?.cached_view().pending);
    assert_eq!(h.id()?, goal);
    Ok(())
}

#[test]
fn room_clock_uses_separate_current_authority_and_refuses_work_before_native_contact() -> Result<()>
{
    for case in 0..6 {
        let mut h = RoomRunHarness::setup(1, 32, false)?;
        match case {
            0 => h
                .room
                .state
                .grants
                .retain(|g| g.capability != Capability::ControlClock),
            1 => h
                .room
                .state
                .room_clock_grants
                .retain(|g| g.capability != Capability::ControlClock),
            2 => h.room.state.budget.max_game_ticks = 0,
            3 => h.room.state.policy.config.checkpoint = runtime::CheckpointPolicy::Required,
            4 => h.io.deny_control = true,
            _ => h.room.state.budget.max_bytes = 1024 * 1024,
        }
        let denied = h.prepare()?;
        assert_eq!(denied["result"]["ok"], false, "case {case}: {denied}");
        assert!(h.io.game.borrow().calls.is_empty(), "case {case}");
        assert_eq!(h.io.acquisitions, 0, "case {case}");
        assert_eq!(h.room.progress()?.observations, 1);
        assert!(h.clock()?.cached_view().runs.is_empty());
    }
    Ok(())
}

#[test]
fn original_furniture_monitor_id_or_foreign_review_cannot_authorize_room_unpause() -> Result<()> {
    let mut h = RoomRunHarness::setup(2, 32, false)?;
    let ordinary_id = h.room.definition()?.original().id();
    let wrong = h.call(room_clock::Action::Prepare(room_clock::Request {
        monitor_id: ordinary_id,
        game_ticks: 10,
        wall_millis: 5000,
    }))?;
    assert_eq!(wrong["result"]["ok"], false, "{wrong}");
    assert!(h.io.game.borrow().calls.is_empty());
    assert_eq!(h.io.acquisitions, 0);
    assert_eq!(h.prepare()?["result"]["ok"], true);
    let correct = h.seal()?;
    let wrong = h.call(room_clock::Action::Commit {
        id: h.id()?,
        seal: Digest32::of_bytes(b"unrelated furniture or placement review"),
    })?;
    assert_eq!(wrong["result"]["ok"], false, "{wrong}");
    assert!(h.clock()?.prepared_seal().is_none());
    let replay = h.call(room_clock::Action::Commit {
        id: h.id()?,
        seal: correct,
    })?;
    assert_eq!(replay["result"]["ok"], false, "{replay}");
    assert!(!h.io.game.borrow().calls.contains(&"commit"));
    Ok(())
}

#[test]
fn room_clock_parent_loss_during_readiness_prevents_unpause_and_keeps_local_cancellation()
-> Result<()> {
    for at_commit in [false, true] {
        let mut h = RoomRunHarness::setup(2, 32, true)?;
        if at_commit {
            assert_eq!(h.prepare()?["result"]["ok"], true);
        }
        h.io.lose_parent_after_sample = h.room.config.batch_path.clone();
        let failed = if at_commit { h.commit()? } else { h.prepare()? };
        assert_eq!(failed["result"]["ok"], false, "{failed}");
        assert!(!h.io.game.borrow().calls.contains(&"commit"));
        assert!(h.clock()?.prepared_seal().is_none());
        assert_eq!(h.room.progress()?.streak, 0);
        let cancelled = h.call(room_clock::Action::Cancel(h.id()?))?;
        assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
        assert!(!h.clock()?.cached_view().pending);
        let monitor_cancelled = h.room.call(room_monitor::Action::Cancel)?;
        assert_eq!(
            monitor_cancelled["result"]["ok"], true,
            "{monitor_cancelled}"
        );
        assert_eq!(h.room.progress()?.phase, "cancelled");
    }
    Ok(())
}

#[test]
fn failed_room_clock_read_consumes_newer_map_time_before_disclosure_authority() -> Result<()> {
    let mut h = RoomRunHarness::setup(2, 32, true)?;
    let goal = h.id()?;
    for grant in &mut h.room.state.grants {
        if grant.capability == Capability::Query {
            grant.expires_at_tick = Some(GameTick(TICK + 100));
        }
    }
    h.io.floor = TICK + 500;
    h.io.fail_sample = true;
    let failed = h.prepare()?;
    assert_eq!(failed["result"]["ok"], false, "{failed}");
    assert!(h.room.state.authority_tick >= TICK + 500);
    assert!(h.room.monitor()?.store.authority_tick_floor() >= TICK + 500);
    assert!(!failed.to_string().contains(&goal.to_string()));
    assert!(!h.io.game.borrow().calls.contains(&"prepare"));
    assert!(!h.io.game.borrow().calls.contains(&"commit"));
    assert_eq!(h.room.progress()?.streak, 0);
    assert!(h.clock()?.prepared_seal().is_none());
    Ok(())
}

#[test]
fn missing_configured_room_clock_or_unverified_furniture_clock_blocks_progress() -> Result<()> {
    let mut h = RoomRunHarness::setup(2, 32, true)?;
    let missing = clock::Config {
        path: h.room.directory.file("unverified-ordinary-clock"),
        initialize: false,
        binding: RunBinding::new(h.room.state.binding.endpoint(), h.io.manifest.clone())?,
        recovery: Some(Recovery {
            journal_id: Digest32::of_bytes(b"missing original ordinary clock"),
            head: Digest32::of_bytes(b"retained ordinary history head"),
        }),
    };
    h.room.config.clock = Some(missing.clone());
    h.room.state.policy.config.clock = Some(missing.clone());
    h.room.state.clock_config = Some(missing);
    assert_eq!(h.prepare()?["result"]["ok"], false);
    assert_eq!(h.io.acquisitions, 0);
    assert!(h.io.game.borrow().calls.is_empty());
    // Use another original owner for loss of its established room clock file.
    let mut h = RoomRunHarness::setup(2, 32, true)?;
    let path = h
        .room
        .state
        .room_clock_config
        .as_ref()
        .ok_or_else(missing_fixture)?
        .path
        .clone();
    fs::remove_file(path).map_err(storage_error)?;
    let before = h.room.io.samples;
    assert_eq!(h.sample()?["result"]["ok"], false);
    assert_eq!(h.room.io.samples, before);
    assert_eq!(h.prepare()?["result"]["ok"], false);
    assert!(h.io.game.borrow().calls.is_empty());
    assert!(h.clock()?.prepared_seal().is_none());
    Ok(())
}

#[test]
fn query_bootstrap_never_mints_plan_authority_or_adopts_an_existing_clock_path() -> Result<()> {
    for substitute in [false, true] {
        let mut h = RoomRunHarness::bootstrap(2, 32, true)?;
        let path = h
            .room
            .state
            .room_clock_config
            .as_ref()
            .ok_or_else(missing_fixture)?
            .path
            .clone();
        assert!(h.room.state.room_clock.is_none());
        assert!(h.room.monitor()?.store.first_source().is_some());
        assert!(!path.exists());
        assert_eq!(h.io.connections, 0);
        let before = h.room.io.samples;
        assert_eq!(h.sample()?["result"]["ok"], false);
        assert_eq!(h.room.io.samples, before);
        if substitute {
            write_private(&path, b"unrelated original clock bytes")?;
        }
        let prepared = h.prepare()?;
        if substitute {
            assert_eq!(prepared["result"]["ok"], false, "{prepared}");
            assert_eq!(
                fs::read(path).map_err(storage_error)?,
                b"unrelated original clock bytes"
            );
            assert!(h.room.state.room_clock.is_none());
            assert!(h.io.game.borrow().calls.is_empty());
        } else {
            assert_eq!(prepared["result"]["ok"], true, "{prepared}");
            assert!(h.room.state.room_clock.is_some());
            assert!(path.exists());
            assert_eq!(h.io.acquisitions, 1);
            assert!(h.clock()?.prepared_seal().is_some());
            assert!(!h.io.game.borrow().calls.contains(&"commit"));
        }
    }
    // A failed initialization can retain an exact recovery pin without an
    // owner. That uncertainty must not strand the independent local monitor.
    let mut h = RoomRunHarness::bootstrap(2, 32, true)?;
    h.io.fail_open = true;
    assert_eq!(h.prepare()?["result"]["ok"], false);
    assert!(h.room.state.room_clock.is_none());
    let pin = h
        .room
        .state
        .room_clock_recovery
        .ok_or_else(missing_fixture)?;
    fs::remove_file(
        h.room
            .config
            .batch_path
            .as_ref()
            .ok_or_else(missing_fixture)?,
    )
    .map_err(storage_error)?;
    let cancelled = h.room.call(room_monitor::Action::Cancel)?;
    assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
    assert_eq!(h.room.progress()?.phase, "cancelled");
    assert_eq!(h.room.state.room_clock_recovery, Some(pin));
    assert_ne!(cancelled["result"]["room_clock"]["history_verified"], true);
    assert!(h.io.game.borrow().calls.is_empty());
    Ok(())
}

#[test]
fn room_clock_final_publication_rechecks_original_monitor_and_run_custody() -> Result<()> {
    let mut baseline = RoomRunHarness::setup(2, 32, true)?;
    let before = baseline.io.checkpoints;
    assert_eq!(baseline.prepare()?["result"]["ok"], true);
    let checkpoints = baseline.io.checkpoints - before;
    assert!(checkpoints > 0);
    for kind in 0..3 {
        let mut h = RoomRunHarness::setup(2, 32, true)?;
        let fault = match kind {
            0 => CheckpointFault::Remove(
                h.room
                    .config
                    .batch_path
                    .clone()
                    .ok_or_else(missing_fixture)?,
            ),
            1 => CheckpointFault::Corrupt(
                h.room
                    .config
                    .room_completion_path
                    .clone()
                    .ok_or_else(missing_fixture)?,
            ),
            _ => CheckpointFault::Corrupt(
                h.room
                    .state
                    .room_clock_config
                    .as_ref()
                    .ok_or_else(missing_fixture)?
                    .path
                    .clone(),
            ),
        };
        h.io.checkpoint_fault = Some(fault);
        h.io.fault_on_checkpoint = Some(h.io.checkpoints + checkpoints);
        let failed = h.prepare()?;
        assert_eq!(failed["result"]["ok"], false, "kind {kind}: {failed}");
        assert!(
            h.io.checkpoint_fault.is_none(),
            "final checkpoint not reached for {kind}"
        );
        assert!(h.clock()?.prepared_seal().is_none());
        assert!(!h.io.game.borrow().calls.contains(&"commit"));
        assert_ne!(
            failed["result"]["room_completion"]["sampled_condition_satisfied"],
            true
        );
        assert_eq!(h.room.progress()?.streak, 0);
    }
    Ok(())
}

#[test]
fn original_room_safety_pause_survives_goal_expiry_and_loss_of_placement_files() -> Result<()> {
    let mut h = RoomRunHarness::setup(2, 32, false)?;
    assert_eq!(h.prepare()?["result"]["ok"], true);
    assert_eq!(h.commit()?["result"]["ok"], true);
    let id = h.id()?;
    let original = h.clock()?.cached_view().runs[0].entry.plan().clone();
    fs::remove_file(&h.room.config.path).map_err(storage_error)?;
    fs::remove_file(
        h.room
            .config
            .batch_path
            .as_ref()
            .ok_or_else(missing_fixture)?,
    )
    .map_err(storage_error)?;
    h.room
        .state
        .grants
        .retain(|g| matches!(g.capability, Capability::Query | Capability::ControlClock));
    h.room
        .state
        .room_clock_grants
        .retain(|g| matches!(g.capability, Capability::Query | Capability::ControlClock));
    h.room.state.budget.max_game_ticks = 0;
    h.io.game.borrow_mut().tick = TICK + 5000;
    let stopped = h.call(room_clock::Action::Cancel(id))?;
    assert_eq!(stopped["result"]["ok"], true, "{stopped}");
    assert!(!h.clock()?.cached_view().pending);
    assert_eq!(h.clock()?.cached_view().runs[0].entry.plan(), &original);
    assert!(h.io.game.borrow().calls.contains(&"cancel"));
    assert_eq!(
        h.io.game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "commit")
            .count(),
        1
    );
    assert!(h.room.state.authority_tick >= TICK + 5000);
    assert_ne!(
        stopped["result"]["room_completion"]["sampled_condition_satisfied"],
        true
    );
    assert_eq!(h.id()?, id);
    Ok(())
}

#[test]
fn public_plan_scope_preserves_placement_and_furniture_clock_and_selects_exact_room() -> Result<()>
{
    let id = Digest32::of_bytes(b"original-public-room-monitor");
    let witness = Digest32::of_bytes(b"original-placement-witness");
    let raw = json!({"monitor_id":id.to_string(),"game_ticks":17,"wall_millis":5000}).to_string();
    for scope in [None, Some("effect")] {
        let action = plan_action(
            scope,
            Some("original-key".into()),
            Some(witness.to_string()),
            None,
        )?;
        assert!(
            matches!(action, Action::Plan { key, witness: actual } if key == "original-key" && actual == witness)
        );
    }
    for scope in [None, Some("clock")] {
        let action = plan_action(scope, None, None, Some(raw.clone()))?;
        assert!(
            matches!(action, Action::Clock(clock::Action::Prepare(request))
            if request.monitor_id == id && request.game_ticks == 17 && request.wall_millis == 5000)
        );
    }
    let action = plan_action(Some("room_clock"), None, None, Some(raw.clone()))?;
    assert!(
        matches!(action, Action::RoomClock(room_clock::Action::Prepare(request))
        if request.monitor_id == id && request.game_ticks == 17 && request.wall_millis == 5000)
    );
    for scope in [
        None,
        Some("effect"),
        Some("clock"),
        Some("room_clock"),
        Some("unknown"),
    ] {
        assert!(
            plan_action(
                scope,
                Some("original-key".into()),
                Some(witness.to_string()),
                Some(raw.clone())
            )
            .is_err()
        );
    }
    for scope in [Some("effect"), Some("unknown")] {
        assert!(plan_action(scope, None, None, Some(raw.clone())).is_err());
    }
    assert!(plan_action(Some("room_clock"), None, None, None).is_err());
    assert!(
        plan_action(
            Some("room_clock"),
            Some("original-key".into()),
            Some(witness.to_string()),
            None
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn public_room_clock_history_query_requires_exact_nonzero_identity_and_closed_page() -> Result<()> {
    let id = Digest32::of_bytes(b"original-room-query").to_string();
    let head = Digest32::of_bytes(b"original-room-clock-head").to_string();
    let value = json!({"mode":"room_clock","monitor_id":id,"head":head});
    assert!(
        matches!(Query::parse(&value.to_string())?, Query::RoomClock { monitor_id, head: actual, offset: None, limit: None }
        if monitor_id == id && actual == head)
    );
    for (offset, limit) in [(0, 1), (1, 1), (63, 8)] {
        let mut page = value.clone();
        page["offset"] = json!(offset);
        page["limit"] = json!(limit);
        assert!(
            matches!(Query::parse(&page.to_string())?, Query::RoomClock { monitor_id, head: actual, offset: Some(o), limit: Some(l) }
            if monitor_id == id && actual == head && o == offset && l == limit)
        );
    }
    for (field, bad) in [
        ("monitor_id", json!(Digest32::ZERO.to_string())),
        ("head", json!(Digest32::ZERO.to_string())),
        ("monitor_id", Value::Null),
        ("head", json!("not-a-head")),
        ("offset", json!(64)),
        ("offset", json!(-1)),
        ("offset", json!(1.0)),
        ("offset", Value::Null),
        ("limit", json!(0)),
        ("limit", json!(9)),
        ("limit", json!(1.0)),
        ("limit", Value::Null),
        ("path", json!("/another/clock")),
    ] {
        let mut changed = value.clone();
        changed[field] = bad;
        assert!(
            Query::parse(&changed.to_string()).is_err(),
            "accepted {field}"
        );
    }
    for field in ["monitor_id", "head"] {
        let mut missing = value.clone();
        missing
            .as_object_mut()
            .ok_or_else(missing_fixture)?
            .remove(field);
        assert!(Query::parse(&missing.to_string()).is_err());
    }
    let duplicate = value.to_string().replacen(
        "\"mode\":\"room_clock\"",
        "\"mode\":\"room_clock\",\"mode\":\"room_clock\"",
        1,
    );
    assert!(Query::parse(&duplicate).is_err());
    assert!(Query::parse(&format!("{}{}", " ".repeat(2048), value)).is_err());
    Ok(())
}
