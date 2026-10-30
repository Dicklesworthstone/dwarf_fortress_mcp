//! Independent restart owns only the exact original completion and clock files.
//! Original placement and parent files are physically removed before opening.
use super::*;
use crate::build_placement_server::clock_recovery::{Mode, RecoveryState};
use dfmcp_adapter::construction_plan::store::Recovery as MonitorRecovery;

type Recovered = RecoveryState<PrivateBuildFile, PrivateRunFile>;

#[derive(Clone, Copy)]
enum History {
    Empty,
    Prepared,
    Dispatched,
    SourceLost,
    Settled,
    MonitorSourceChanged,
}

struct RecoveryIo {
    inner: ClockIo,
    replace_after_cancel: Option<(PathBuf, PathBuf)>,
    unexpected_opens: usize,
}
impl clock::ClockIo for RecoveryIo {
    type Source = RunNative;

    fn opening_recovery(&self) -> Option<Recovery> {
        self.inner.opening
    }
    fn open(
        &mut self,
        _: &clock::Config,
        _: &MonitorDefinition,
        _: &Manifest,
        _: &OperationContext,
        _: &OperationContext,
    ) -> Result<Clock<PrivateRunFile>> {
        self.unexpected_opens += 1;
        Err(error(
            ErrorCode::InternalInvariantViolation,
            "already opened recovery attempted another clock owner",
        ))
    }
    fn connect(
        &mut self,
        config: &clock::Config,
        c: &OperationContext,
        control: bool,
    ) -> Result<Self::Source> {
        clock::ClockIo::connect(&mut self.inner, config, c, control)
    }
    fn sample(&mut self, _: &MonitorDefinition, _: &OperationContext) -> Result<LinkedSample> {
        self.inner.acquisitions += 1;
        Err(error(
            ErrorCode::InternalInvariantViolation,
            "independent recovery attempted a fresh construction acquisition",
        ))
    }
    fn checkpoint(&mut self, c: &OperationContext, control: bool) -> Result<()> {
        clock::ClockIo::checkpoint(&mut self.inner, c, control)?;
        if self.inner.game.borrow().calls.last() == Some(&"cancel") {
            if let Some((path, backup)) = self.replace_after_cancel.take() {
                let bytes = fs::read(&path).map_err(storage_error)?;
                fs::rename(&path, backup).map_err(storage_error)?;
                write_private(&path, &bytes)?;
            }
        }
        Ok(())
    }
}

struct Retained {
    config: Config,
    canonical: OperationContext,
    monitor_pin: MonitorRecovery,
    ancestor_monitor_pin: MonitorRecovery,
    clock_pin: Recovery,
    ancestor_clock_pin: Recovery,
    monitor_before: Vec<u8>,
    clock_before: Vec<u8>,
    io: RecoveryIo,
    directory: Directory,
    calls_before: Vec<&'static str>,
    connections_before: usize,
    acquisitions_before: usize,
    original_count: usize,
}
impl Retained {
    fn contexts(&self, mode: Mode, enabled: bool) -> Result<(OperationContext, OperationContext)> {
        let mut config = self.config.clone();
        config.mode = if mode == Mode::Offline {
            BuildMode::Offline
        } else {
            BuildMode::Recover
        };
        let (grants, native) = clock_recovery_authority(&config, enabled);
        let mut c = self.canonical.clone();
        c.grants = grants;
        c.budget.max_game_ticks = 0;
        let n = clock::native_context(&c, &native)?;
        Ok((c, n))
    }
    fn open(&self, mode: Mode, enabled: bool) -> Result<Recovered> {
        let (c, n) = self.contexts(mode, enabled)?;
        let run_mode = if mode == Mode::Offline {
            RunMode::Offline
        } else if enabled {
            RunMode::Control
        } else {
            RunMode::Recover
        };
        self.open_with(mode, run_mode, &c, &n, &self.config)
    }
    fn open_with(
        &self,
        mode: Mode,
        run_mode: RunMode,
        c: &OperationContext,
        n: &OperationContext,
        config: &Config,
    ) -> Result<Recovered> {
        let mut config = config.clone();
        config.mode = if mode == Mode::Offline {
            BuildMode::Offline
        } else {
            BuildMode::Recover
        };
        let mut monitor = open_private_monitor(
            config
                .completion_path
                .as_ref()
                .ok_or_else(missing_fixture)?,
            c,
            BuildMode::Offline,
            None,
        )?;
        monitor.verify_recovery(
            c,
            config.clock_monitor_recovery.ok_or_else(missing_fixture)?,
        )?;
        let operations = monitor
            .first_source()
            .ok_or_else(missing_fixture)?
            .operations
            .clone();
        let selected = config.clock.as_ref().ok_or_else(missing_fixture)?;
        let journal = open_private_run_journal_exact(
            &selected.path,
            n,
            run_mode,
            Some(selected.binding.clone()),
            false,
        )?;
        let clock = Clock::new(
            journal,
            monitor.definition().clone(),
            operations,
            selected.recovery,
            c,
            n,
        )?;
        RecoveryState::new(monitor, clock, c, n, &config, mode)
    }
    fn isolated(&self) {
        assert!(!self.config.path.exists());
        assert!(self.config.batch_path.as_ref().is_some_and(|p| !p.exists()));
        assert_eq!(self.io.inner.acquisitions, self.acquisitions_before);
        assert_eq!(self.io.unexpected_opens, 0);
        assert_eq!(
            self.io
                .inner
                .game
                .borrow()
                .calls
                .iter()
                .filter(|&&v| v == "commit")
                .count(),
            self.calls_before.iter().filter(|&&v| v == "commit").count()
        );
        assert_eq!(
            self.io
                .inner
                .game
                .borrow()
                .calls
                .iter()
                .filter(|&&v| v == "prepare")
                .count(),
            self.calls_before
                .iter()
                .filter(|&&v| v == "prepare")
                .count()
        );
        assert_eq!(
            self.io
                .inner
                .game
                .borrow()
                .calls
                .iter()
                .filter(|&&v| v == "observe")
                .count(),
            self.calls_before
                .iter()
                .filter(|&&v| v == "observe")
                .count()
        );
    }
    fn monitor_path(&self) -> Result<&Path> {
        self.config
            .completion_path
            .as_deref()
            .ok_or_else(missing_fixture)
    }
    fn clock_path(&self) -> Result<&Path> {
        Ok(&self.config.clock.as_ref().ok_or_else(missing_fixture)?.path)
    }
}

fn retained(history: History, count: usize) -> Result<Retained> {
    let mut h = setup_clock(count)?;
    let ancestor_monitor_pin = h
        .state
        .completion
        .as_ref()
        .ok_or_else(missing_fixture)?
        .store
        .recovery();
    let ancestor_clock_pin = h
        .state
        .clock
        .as_ref()
        .ok_or_else(missing_fixture)?
        .recovery();
    let monitor_before = fs::read(
        h.config
            .completion_path
            .as_ref()
            .ok_or_else(missing_fixture)?,
    )
    .map_err(storage_error)?;
    let clock_before = fs::read(&h.config.clock.as_ref().ok_or_else(missing_fixture)?.path)
        .map_err(storage_error)?;
    // Preserve an acknowledged ancestor as well as a later complete sample.
    let c = h.state.context(false, None)?;
    clock::ensure_for_sample(&mut h.state, &c)?;
    let monitor = h.state.completion.as_mut().ok_or_else(missing_fixture)?;
    let sample = linked_sample(monitor.store.definition(), TICK + 2, false)?;
    let mut admit = |_: &MonitorDefinition, _: &Progress| Ok(());
    let mut guard = |_: &Origin, _: &OperationContext| Ok(());
    monitor.store.begin_read(&c, &mut admit, &mut guard)?;
    monitor
        .store
        .publish_sample(&c, &sample, &mut admit, &mut guard)?;
    h.io.game.borrow_mut().tick = TICK + 2;
    if !matches!(history, History::Empty) {
        success(&h.prepare()?);
        if !matches!(history, History::Prepared) {
            let seal = h.seal()?;
            h.io.game.borrow_mut().lose_commit = true;
            assert_eq!(h.commit(seal)?["result"]["ok"], false);
            if matches!(history, History::SourceLost) {
                let mut game = h.io.game.borrow_mut();
                let plan = game
                    .record
                    .as_ref()
                    .ok_or_else(missing_fixture)?
                    .plan()
                    .clone();
                game.record = Some(run_record(&plan, 5, 7, 0)?);
                drop(game);
                success(&h.call(clock::Action::Wait(h.id()?))?);
            } else if matches!(history, History::Settled) {
                h.io.game.borrow_mut().finish()?;
                success(&h.call(clock::Action::Wait(h.id()?))?);
            }
        }
    }
    if matches!(history, History::MonitorSourceChanged) {
        // A later valid monitor history can invalidate its operations source.
        // Reopening must still recover the old clock's original association.
        let c = h.state.context(false, None)?;
        let monitor = h.state.completion.as_mut().ok_or_else(missing_fixture)?;
        let original = monitor
            .store
            .first_source()
            .ok_or_else(missing_fixture)?
            .clone();
        let mut sample = linked_sample(monitor.store.definition(), TICK + 3, false)?;
        let mut observed = LiveOperationsObservation::decode_profile(
            &sample.capture,
            sample.operations.generation,
            sample.operations.df_version.clone(),
            sample.operations.dfhack_version.clone(),
            OperationsProfile::PagedV1_4,
        )?;
        sample.operations.generation += 1;
        observed.jobs.bridge_generation = sample.operations.generation;
        sample.capture = observed.encode_profile(OperationsProfile::PagedV1_4)?;
        monitor.store.begin_read(&c, &mut admit, &mut guard)?;
        monitor
            .store
            .publish_sample(&c, &sample, &mut admit, &mut guard)?;
        assert_eq!(monitor.store.progress().phase, "invalidated");
        assert_eq!(monitor.store.progress().reason, "native_source_changed");
        assert_eq!(monitor.store.first_source(), Some(&original));
        assert_ne!(monitor.store.progress().source.as_ref(), Some(&original));
    }
    let monitor_pin = h
        .state
        .completion
        .as_ref()
        .ok_or_else(missing_fixture)?
        .store
        .recovery();
    let clock_pin = h
        .state
        .clock
        .as_ref()
        .ok_or_else(missing_fixture)?
        .recovery();
    let mut canonical = h.state.context(false, None)?;
    canonical.session_id = SessionId::new(30_000 + count as u128);
    canonical.request_id = RequestId::new(1);
    canonical.anchor.tick = GameTick(0);
    canonical.budget.max_game_ticks = 0;
    let Harness {
        state,
        mut config,
        io,
        directory,
    } = h;
    drop(state);
    fs::remove_file(&config.path).map_err(storage_error)?;
    fs::remove_file(config.batch_path.as_ref().ok_or_else(missing_fixture)?)
        .map_err(storage_error)?;
    config.clock_only = true;
    config.completion_only = false;
    config.mode = BuildMode::Recover;
    config.checkpoint = runtime::CheckpointPolicy::Required;
    config.clock_monitor_recovery = Some(monitor_pin);
    let selected = config.clock.as_mut().ok_or_else(missing_fixture)?;
    selected.initialize = false;
    selected.recovery = Some(clock_pin);
    let calls_before = io.game.borrow().calls.clone();
    let connections_before = io.connections;
    let acquisitions_before = io.acquisitions;
    Ok(Retained {
        config,
        canonical,
        monitor_pin,
        ancestor_monitor_pin,
        clock_pin,
        ancestor_clock_pin,
        monitor_before,
        clock_before,
        io: RecoveryIo {
            inner: io,
            replace_after_cancel: None,
            unexpected_opens: 0,
        },
        directory,
        calls_before,
        connections_before,
        acquisitions_before,
        original_count: count,
    })
}

fn decoded(raw: String) -> Result<Value> {
    assert!(raw.len() as u64 <= OUTPUT_BYTES);
    serde_json::from_str(&raw).map_err(|_| missing_fixture())
}
fn success(value: &Value) {
    assert_eq!(value["result"]["ok"], true, "{}", value["result"]["error"]);
}
fn dispatch(state: &mut Recovered, io: &mut RecoveryIo, action: Action) -> Result<Value> {
    let op = match &action {
        Action::Clock(clock::Action::Wait(_)) => "fortress.wait",
        Action::Clock(clock::Action::Cancel(_)) => "fortress.cancel",
        Action::Clock(clock::Action::Commit { .. }) | Action::Commit { .. } => "fortress.commit",
        Action::Clock(clock::Action::Prepare(_)) | Action::Plan { .. } => "fortress.plan",
        Action::Observe(_)
        | Action::ObserveNext
        | Action::Completion(completion::Action::Sample) => "fortress.observe",
        Action::Inventory => "fortress.doctor",
        _ => "fortress.query",
    };
    let c = state.context(None)?;
    let n = state.native_context(&c)?;
    decoded(state.run_action(c, n, op, Ok(action), Instant::now(), io))
}
fn close(state: &mut Recovered, release: bool) -> Result<Value> {
    let c = state.context(None)?;
    let n = state.native_context(&c)?;
    decoded(state.close(c, n, release, Instant::now(), |_| Ok(())))
}
fn clock_query(state: &Recovered, id: Digest32) -> Result<Action> {
    let head = digest(
        state.clock_reference()["recovery"]["head"]
            .as_str()
            .ok_or_else(missing_fixture)?,
    )?;
    Ok(Action::Clock(clock::Action::Inspect {
        id,
        head,
        offset: 0,
        limit: 8,
    }))
}
fn assert_history(value: &Value, expected: usize) {
    assert_eq!(value["result"]["clock_recovery_only"], true);
    assert_eq!(value["result"]["batch"]["inventory_verified"], false);
    assert_eq!(value["result"]["batch"]["total"], expected);
    assert_eq!(
        value["result"]["construction_clock"]["current_pause_proven"],
        false
    );
    assert_eq!(
        value["result"]["construction_clock"]["fresh_goal_sampling_permitted"],
        false
    );
    assert_eq!(
        value["result"]["construction_clock"]["sampled_construction_condition_proven"],
        false
    );
    assert_eq!(value["result"]["new_run_permitted"], false);
    assert_eq!(value["result"]["retry_commit_permitted"], false);
    if let Some(recommendations) = value["agent_turn"]["recommendations"].as_array() {
        for recommendation in recommendations {
            assert!(!matches!(
                recommendation["tool"].as_str(),
                Some("fortress.observe" | "fortress.plan" | "fortress.commit")
            ));
        }
    }
}
fn assert_affordance(value: &Value, tool: Option<&str>, cancellation: bool) {
    assert_eq!(
        value["agent_turn"]["recommendations"]
            .as_array()
            .map(Vec::len),
        Some(usize::from(tool.is_some()))
    );
    if let Some(tool) = tool {
        assert_eq!(value["agent_turn"]["recommendations"][0]["tool"], tool);
    }
    assert_eq!(
        !value["agent_turn"]["active_work"]["construction_clock"]["cancellation"].is_null(),
        cancellation
    );
}
fn forbid_disclosure(value: &Value) {
    assert_eq!(value["result"]["ok"], false);
    for key in ["batch", "completion", "construction_clock"] {
        assert!(
            value["result"].get(key).is_none(),
            "unauthorized {key} disclosure"
        );
    }
}

#[test]
fn independent_offline_restart_keeps_every_original_member_without_native_or_parent_custody()
-> Result<()> {
    for history in [
        History::Prepared,
        History::Dispatched,
        History::SourceLost,
        History::Settled,
    ] {
        let mut evidence = retained(history, 3)?;
        let monitor_bytes = fs::read(evidence.monitor_path()?).map_err(storage_error)?;
        let clock_bytes = fs::read(evidence.clock_path()?).map_err(storage_error)?;
        let mut owner = evidence.open(Mode::Offline, false)?;
        let id = evidence.monitor_pin.monitor_id;
        let query = clock_query(&owner, id)?;
        let inspected = dispatch(&mut owner, &mut evidence.io, query)?;
        success(&inspected);
        assert_history(&inspected, evidence.original_count);
        assert_affordance(&inspected, Some("fortress.query"), false);
        for action in [
            Action::Completion(completion::Action::Inspect),
            Action::Query(Query::Batch {}),
            Action::Inventory,
            Action::Query(Query::Schema {}),
        ] {
            success(&dispatch(&mut owner, &mut evidence.io, action)?);
        }
        for action in [
            Action::Clock(clock::Action::Wait(id)),
            Action::Clock(clock::Action::Cancel(id)),
        ] {
            assert_eq!(
                dispatch(&mut owner, &mut evidence.io, action)?["result"]["ok"],
                false
            );
        }
        assert_eq!(evidence.io.inner.game.borrow().calls, evidence.calls_before);
        assert_eq!(evidence.io.inner.connections, evidence.connections_before);
        assert_eq!(
            fs::read(evidence.monitor_path()?).map_err(storage_error)?,
            monitor_bytes
        );
        assert_eq!(
            fs::read(evidence.clock_path()?).map_err(storage_error)?,
            clock_bytes
        );
        let released = close(&mut owner, true)?;
        success(&released);
        assert_affordance(&released, None, false);
        evidence.isolated();
    }
    Ok(())
}

#[test]
fn lost_dispatch_restart_reconciles_and_safety_cancels_only_the_original_interval() -> Result<()> {
    let mut evidence = retained(History::Dispatched, 3)?;
    let mut owner = evidence.open(Mode::Recovery, true)?;
    assert_eq!(evidence.io.inner.connections, evidence.connections_before);
    let id = evidence.monitor_pin.monitor_id;
    let running = dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Clock(clock::Action::Wait(id)),
    )?;
    success(&running);
    assert_history(&running, 3);
    assert_affordance(&running, Some("fortress.wait"), true);
    assert_eq!(running["result"]["construction_clock"]["pending"], true);
    let cancelled = dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Clock(clock::Action::Cancel(id)),
    )?;
    success(&cancelled);
    assert_history(&cancelled, 3);
    assert_affordance(&cancelled, Some("fortress.query"), false);
    assert_eq!(cancelled["result"]["construction_clock"]["pending"], false);
    assert_eq!(
        evidence
            .io
            .inner
            .game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "query")
            .count(),
        1
    );
    assert_eq!(
        evidence
            .io
            .inner
            .game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "cancel")
            .count(),
        1
    );
    let connections = evidence.io.inner.connections;
    success(&dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Clock(clock::Action::Cancel(id)),
    )?);
    assert_eq!(evidence.io.inner.connections, connections);
    let closed = close(&mut owner, false)?;
    success(&closed);
    assert_affordance(&closed, None, false);
    evidence.isolated();
    Ok(())
}

#[test]
fn prepared_restart_has_no_review_and_can_retire_without_native_credentials() -> Result<()> {
    let mut evidence = retained(History::Prepared, 3)?;
    let mut owner = evidence.open(Mode::Recovery, true)?;
    let id = evidence.monitor_pin.monitor_id;
    let query = clock_query(&owner, id)?;
    let inspected = dispatch(&mut owner, &mut evidence.io, query)?;
    success(&inspected);
    assert!(inspected["result"]["construction_clock"]["active_run"]["review_seal"].is_null());
    let refused = dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Clock(clock::Action::Commit {
            id,
            seal: Digest32::of_bytes(b"prior process review cannot be recreated"),
        }),
    )?;
    assert_eq!(refused["result"]["ok"], false);
    success(&dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Clock(clock::Action::Cancel(id)),
    )?);
    assert_eq!(evidence.io.inner.connections, evidence.connections_before);
    assert_eq!(evidence.io.inner.game.borrow().calls, evidence.calls_before);
    success(&close(&mut owner, false)?);
    evidence.isolated();
    Ok(())
}

#[test]
fn query_only_recovery_cannot_gain_safety_authority_from_current_broader_grants() -> Result<()> {
    for supplied_opening_clock in [false, true] {
        let mut evidence = retained(History::Dispatched, 3)?;
        let (c, n) = evidence.contexts(Mode::Recovery, supplied_opening_clock)?;
        let mut owner =
            evidence.open_with(Mode::Recovery, RunMode::Recover, &c, &n, &evidence.config)?;
        let id = evidence.monitor_pin.monitor_id;
        let running = dispatch(
            &mut owner,
            &mut evidence.io,
            Action::Clock(clock::Action::Wait(id)),
        )?;
        success(&running);
        assert_affordance(&running, Some("fortress.wait"), false);
        let mut current = owner.context(None)?;
        let (_, broad_native) = evidence.contexts(Mode::Recovery, true)?;
        let (broad_canonical, _) = evidence.contexts(Mode::Recovery, true)?;
        current.grants = broad_canonical.grants;
        let mut native = owner.native_context(&current)?;
        native.grants = broad_native.grants;
        let calls = evidence.io.inner.connections;
        let denied = decoded(owner.run_action(
            current,
            native,
            "fortress.cancel",
            Ok(Action::Clock(clock::Action::Cancel(id))),
            Instant::now(),
            &mut evidence.io,
        ))?;
        assert_eq!(denied["result"]["ok"], false);
        assert_eq!(evidence.io.inner.connections, calls);
        assert!(!evidence.io.inner.game.borrow().calls.contains(&"cancel"));
        let c = owner.context(None)?;
        assert!(
            c.grants
                .iter()
                .all(|grant| grant.capability == Capability::Query)
        );
        assert_eq!(c.budget.max_game_ticks, 0);
        success(&close(&mut owner, true)?);
        evidence.isolated();
    }
    Ok(())
}

#[test]
fn independent_owner_refuses_placement_sampling_new_clock_and_monitor_mutation() -> Result<()> {
    let mut evidence = retained(History::Prepared, 3)?;
    let mut owner = evidence.open(Mode::Recovery, true)?;
    let id = evidence.monitor_pin.monitor_id;
    let actions = [
        Action::ObserveNext,
        Action::StopBatch,
        Action::Completion(completion::Action::Sample),
        Action::Completion(completion::Action::Cancel),
        Action::Completion(completion::Action::Start(Timing {
            deadline: TICK + 5000,
            interval: 1,
            stable_samples: 2,
            stable_span: 1,
            max_gap: 1200,
            max_observations: 32,
        })),
        Action::Clock(clock::Action::Prepare(clock::Request {
            monitor_id: id,
            game_ticks: 10,
            wall_millis: 5000,
        })),
        Action::Query(Query::Records {
            offset: None,
            limit: None,
            head: None,
        }),
        Action::Denied,
    ];
    for action in actions {
        let mut c = owner.context(None)?;
        c.grants.extend(evidence.canonical.grants.clone());
        c.budget.max_game_ticks = 1200;
        let n = owner.native_context(&c)?;
        let denied = decoded(owner.run_action(
            c,
            n,
            "fortress.plan",
            Ok(action),
            Instant::now(),
            &mut evidence.io,
        ))?;
        assert_eq!(denied["result"]["ok"], false);
    }
    assert_eq!(evidence.io.inner.connections, evidence.connections_before);
    assert_eq!(evidence.io.inner.game.borrow().calls, evidence.calls_before);
    evidence.isolated();
    Ok(())
}

#[test]
fn monitor_damage_after_open_does_not_strand_original_clock_safety() -> Result<()> {
    let mut evidence = retained(History::Dispatched, 3)?;
    let mut owner = evidence.open(Mode::Recovery, true)?;
    let id = evidence.monitor_pin.monitor_id;
    fs::remove_file(evidence.monitor_path()?).map_err(storage_error)?;
    let monitor = dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Completion(completion::Action::Inspect),
    )?;
    assert_eq!(monitor["result"]["ok"], false);
    let cancelled = dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Clock(clock::Action::Cancel(id)),
    )?;
    success(&cancelled);
    assert_history(&cancelled, 3);
    assert_ne!(
        cancelled["result"]["completion"]["monitor_inventory_verified"],
        true
    );
    assert_eq!(cancelled["result"]["construction_clock"]["pending"], false);
    assert_eq!(
        evidence
            .io
            .inner
            .game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "cancel")
            .count(),
        1
    );
    success(&close(&mut owner, true)?);
    evidence.isolated();
    Ok(())
}

#[test]
fn clock_damage_blocks_native_connections_and_requires_explicit_recovery_release() -> Result<()> {
    for replace in [false, true] {
        let mut evidence = retained(History::Dispatched, 3)?;
        let mut owner = evidence.open(Mode::Recovery, true)?;
        let path = evidence.clock_path()?.to_path_buf();
        let mut bytes = fs::read(&path).map_err(storage_error)?;
        if replace {
            fs::rename(&path, evidence.directory.file("old-clock-inode")).map_err(storage_error)?;
        } else {
            *bytes.first_mut().ok_or_else(missing_fixture)? ^= 1;
        }
        write_private(&path, &bytes)?;
        for action in [
            Action::Clock(clock::Action::Wait(evidence.monitor_pin.monitor_id)),
            Action::Clock(clock::Action::Cancel(evidence.monitor_pin.monitor_id)),
        ] {
            assert_eq!(
                dispatch(&mut owner, &mut evidence.io, action)?["result"]["ok"],
                false
            );
        }
        assert_eq!(evidence.io.inner.connections, evidence.connections_before);
        assert!(!evidence.io.inner.game.borrow().calls.contains(&"cancel"));
        assert_eq!(close(&mut owner, false)?["result"]["ok"], false);
        let released = close(&mut owner, true)?;
        success(&released);
        assert_eq!(released["result"]["release_for_recovery"], true);
        assert_eq!(
            released["result"]["construction_clock"]["history_verified"],
            false
        );
        assert_eq!(
            released["result"]["construction_clock"]["recovery"]["journal_id"],
            evidence.clock_pin.journal_id.to_string()
        );
        evidence.isolated();
    }
    Ok(())
}

#[test]
fn absent_native_history_and_source_lost_never_prove_pause_or_unlock_a_new_interval() -> Result<()>
{
    for source_lost in [false, true] {
        let history = if source_lost {
            History::SourceLost
        } else {
            History::Dispatched
        };
        let mut evidence = retained(history, 3)?;
        let mut owner = evidence.open(Mode::Recovery, true)?;
        evidence.io.inner.game.borrow_mut().absent = !source_lost;
        let id = evidence.monitor_pin.monitor_id;
        let result = dispatch(
            &mut owner,
            &mut evidence.io,
            Action::Clock(clock::Action::Wait(id)),
        )?;
        assert_eq!(result["result"]["construction_clock"]["unresolved"], true);
        assert_history(&result, 3);
        if source_lost {
            success(&result);
            success(&dispatch(
                &mut owner,
                &mut evidence.io,
                Action::Clock(clock::Action::Cancel(id)),
            )?);
            assert_eq!(evidence.io.inner.connections, evidence.connections_before);
        }
        assert_eq!(close(&mut owner, false)?["result"]["ok"], false);
        let released = close(&mut owner, true)?;
        success(&released);
        assert_eq!(released["result"]["construction_clock"]["unresolved"], true);
        assert!(!evidence.io.inner.game.borrow().calls.contains(&"cancel"));
        evidence.isolated();
    }
    Ok(())
}

#[test]
fn exact_monitor_and_clock_ancestors_accept_lost_acknowledgements_without_adopting_rollback()
-> Result<()> {
    let evidence = retained(History::Dispatched, 3)?;
    let (c, n) = evidence.contexts(Mode::Recovery, true)?;
    let mut config = evidence.config.clone();
    config.clock_monitor_recovery = Some(evidence.ancestor_monitor_pin);
    config.clock.as_mut().ok_or_else(missing_fixture)?.recovery = Some(evidence.ancestor_clock_pin);
    let owner = evidence.open_with(Mode::Recovery, RunMode::Control, &c, &n, &config)?;
    assert_eq!(
        owner.monitor_reference()["recovery"]["head"],
        evidence.monitor_pin.head.to_string()
    );
    assert_eq!(
        owner.clock_reference()["recovery"]["head"],
        evidence.clock_pin.head.to_string()
    );
    drop(owner);
    for clock_rollback in [false, true] {
        let evidence = retained(History::Dispatched, 3)?;
        if clock_rollback {
            write_private(evidence.clock_path()?, &evidence.clock_before)?;
        } else {
            write_private(evidence.monitor_path()?, &evidence.monitor_before)?;
        }
        assert!(evidence.open(Mode::Recovery, true).is_err());
        assert_eq!(evidence.io.inner.connections, evidence.connections_before);
        evidence.isolated();
    }
    Ok(())
}

#[test]
fn startup_rejects_missing_pins_substitution_source_changes_and_new_initialization() -> Result<()> {
    let evidence = retained(History::Prepared, 3)?;
    let (c, n) = evidence.contexts(Mode::Recovery, true)?;
    for case in 0..7 {
        let mut config = evidence.config.clone();
        match case {
            0 => config.clock_monitor_recovery = None,
            1 => {
                config
                    .clock_monitor_recovery
                    .as_mut()
                    .ok_or_else(missing_fixture)?
                    .monitor_id = Digest32::of_bytes(b"other original furniture goal")
            }
            2 => {
                config
                    .clock_monitor_recovery
                    .as_mut()
                    .ok_or_else(missing_fixture)?
                    .head = Digest32::of_bytes(b"other monitor history")
            }
            3 => config.clock.as_mut().ok_or_else(missing_fixture)?.recovery = None,
            4 => {
                config
                    .clock
                    .as_mut()
                    .ok_or_else(missing_fixture)?
                    .initialize = true
            }
            5 => {
                config
                    .clock
                    .as_mut()
                    .ok_or_else(missing_fixture)?
                    .recovery
                    .as_mut()
                    .ok_or_else(missing_fixture)?
                    .journal_id = Digest32::of_bytes(b"other clock journal")
            }
            _ => {
                let selected = config.clock.as_mut().ok_or_else(missing_fixture)?;
                let mut manifest = selected.binding.manifest().clone();
                manifest.generation += 1;
                selected.binding = RunBinding::new(selected.binding.endpoint(), manifest)?;
            }
        }
        assert!(
            evidence
                .open_with(Mode::Recovery, RunMode::Control, &c, &n, &config)
                .is_err(),
            "case {case}"
        );
    }
    let mut c = c.clone();
    let mut n = n.clone();
    c.budget.max_game_ticks = 1;
    n.budget.max_game_ticks = 1;
    assert!(
        evidence
            .open_with(Mode::Recovery, RunMode::Control, &c, &n, &evidence.config)
            .is_err()
    );
    evidence.isolated();
    Ok(())
}

#[test]
fn startup_requires_intact_original_monitor_but_never_recreates_it() -> Result<()> {
    for remove in [false, true] {
        let evidence = retained(History::Dispatched, 3)?;
        let path = evidence.monitor_path()?.to_path_buf();
        if remove {
            fs::remove_file(&path).map_err(storage_error)?;
        } else {
            let mut bytes = fs::read(&path).map_err(storage_error)?;
            bytes.pop();
            write_private(&path, &bytes)?;
        }
        assert!(evidence.open(Mode::Recovery, true).is_err());
        assert_eq!(path.exists(), !remove);
        assert_eq!(evidence.io.inner.connections, evidence.connections_before);
        evidence.isolated();
    }
    Ok(())
}

#[test]
fn each_independent_current_query_can_refuse_cached_recovery_disclosure() -> Result<()> {
    for native in [false, true] {
        let mut evidence = retained(History::Prepared, 3)?;
        let mut owner = evidence.open(Mode::Recovery, true)?;
        let mut c = owner.context(None)?;
        let mut n = owner.native_context(&c)?;
        if native {
            n.grants.clear();
        } else {
            c.grants.clear();
        }
        let action = clock_query(&owner, evidence.monitor_pin.monitor_id)?;
        let result = decoded(owner.run_action(
            c,
            n,
            "fortress.query",
            Ok(action),
            Instant::now(),
            &mut evidence.io,
        ))?;
        forbid_disclosure(&result);
        assert_eq!(evidence.io.inner.connections, evidence.connections_before);
        evidence.isolated();
    }
    Ok(())
}

#[test]
fn consumed_receipt_tick_expires_retained_query_without_rewind_or_replacement_authority()
-> Result<()> {
    for native_expiry in [false, true] {
        let mut evidence = retained(History::Dispatched, 3)?;
        let (mut c, mut n) = evidence.contexts(Mode::Recovery, true)?;
        let grants = if native_expiry {
            &mut n.grants
        } else {
            &mut c.grants
        };
        for grant in grants
            .iter_mut()
            .filter(|grant| grant.capability == Capability::Query)
        {
            grant.expires_at_tick = Some(GameTick(TICK + 5));
        }
        let mut owner =
            evidence.open_with(Mode::Recovery, RunMode::Control, &c, &n, &evidence.config)?;
        let learned = TICK + 20;
        {
            let mut game = evidence.io.inner.game.borrow_mut();
            let plan = game
                .record
                .as_ref()
                .ok_or_else(missing_fixture)?
                .plan()
                .clone();
            game.tick = learned;
            game.paused = true;
            game.record = Some(run_record(&plan, 3, 1, learned)?);
        }
        let result = dispatch(
            &mut owner,
            &mut evidence.io,
            Action::Clock(clock::Action::Wait(evidence.monitor_pin.monitor_id)),
        )?;
        forbid_disclosure(&result);
        assert!(owner.high_tick() >= learned);
        let calls = evidence.io.inner.connections;
        let mut current = owner.context(None)?;
        let (new_c, new_n) = evidence.contexts(Mode::Recovery, true)?;
        current.grants = new_c.grants;
        current.anchor.tick = GameTick(0);
        let mut native = owner.native_context(&current)?;
        native.grants = new_n.grants;
        native.anchor.tick = GameTick(0);
        let result = decoded(owner.run_action(
            current,
            native,
            "fortress.cancel",
            Ok(Action::Clock(clock::Action::Cancel(
                evidence.monitor_pin.monitor_id,
            ))),
            Instant::now(),
            &mut evidence.io,
        ))?;
        forbid_disclosure(&result);
        assert!(owner.high_tick() >= learned);
        assert_eq!(evidence.io.inner.connections, calls);
        assert!(!evidence.io.inner.game.borrow().calls.contains(&"cancel"));
        let released = close(&mut owner, true)?;
        success(&released);
        assert_eq!(released["result"]["closed"], true);
        assert_eq!(released["result"]["release_for_recovery"], true);
        assert_affordance(&released, None, false);
        for key in ["batch", "completion", "construction_clock"] {
            assert!(
                released["result"].get(key).is_none(),
                "release leaked {key}"
            );
        }
        assert_eq!(evidence.io.inner.connections, calls);
        evidence.isolated();
    }
    Ok(())
}

#[test]
fn cancellation_still_works_after_goal_deadline_with_no_future_work_or_placement_permission()
-> Result<()> {
    let mut evidence = retained(History::Dispatched, 3)?;
    let mut owner = evidence.open(Mode::Recovery, true)?;
    let mut c = owner.context(None)?;
    c.anchor.tick = GameTick(TICK + 2000);
    c.budget.max_game_ticks = 0;
    let n = owner.native_context(&c)?;
    assert!(
        c.grants
            .iter()
            .all(|g| matches!(g.capability, Capability::Query | Capability::ControlClock))
    );
    assert!(
        n.grants
            .iter()
            .all(|g| matches!(g.capability, Capability::Query | Capability::ControlClock))
    );
    evidence.io.inner.game.borrow_mut().tick = TICK + 2000;
    let result = decoded(owner.run_action(
        c,
        n,
        "fortress.cancel",
        Ok(Action::Clock(clock::Action::Cancel(
            evidence.monitor_pin.monitor_id,
        ))),
        Instant::now(),
        &mut evidence.io,
    ))?;
    success(&result);
    assert!(owner.high_tick() >= TICK + 2000);
    assert_history(&result, 3);
    assert_eq!(
        evidence
            .io
            .inner
            .game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "cancel")
            .count(),
        1
    );
    evidence.isolated();
    Ok(())
}

#[test]
fn final_publication_rejects_replaced_clock_after_safety_receipt_without_repeating_effect()
-> Result<()> {
    let mut evidence = retained(History::Dispatched, 3)?;
    let mut owner = evidence.open(Mode::Recovery, true)?;
    evidence.io.replace_after_cancel = Some((
        evidence.clock_path()?.to_path_buf(),
        evidence.directory.file("clock-before-final-check"),
    ));
    let result = dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Clock(clock::Action::Cancel(evidence.monitor_pin.monitor_id)),
    )?;
    assert_eq!(result["result"]["ok"], false);
    assert_eq!(result["result"]["effect_may_have_occurred"], true);
    assert_eq!(
        result["result"]["construction_clock"]["history_verified"],
        false
    );
    assert_eq!(
        evidence
            .io
            .inner
            .game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "cancel")
            .count(),
        1
    );
    let calls = evidence.io.inner.connections;
    assert_eq!(
        dispatch(
            &mut owner,
            &mut evidence.io,
            Action::Clock(clock::Action::Cancel(evidence.monitor_pin.monitor_id))
        )?["result"]["ok"],
        false
    );
    assert_eq!(evidence.io.inner.connections, calls);
    success(&close(&mut owner, true)?);
    evidence.isolated();
    Ok(())
}

#[test]
fn head_pinned_pages_and_maximum_original_set_survive_independent_restart() -> Result<()> {
    let mut evidence = retained(History::Prepared, 32)?;
    let mut owner = evidence.open(Mode::Offline, false)?;
    let query = clock_query(&owner, evidence.monitor_pin.monitor_id)?;
    let result = dispatch(&mut owner, &mut evidence.io, query)?;
    success(&result);
    assert_history(&result, 32);
    assert_eq!(
        result["result"]["batch"]["steps"].as_array().map(Vec::len),
        Some(32)
    );
    assert_eq!(
        result["result"]["construction_clock"]["runs_page"]["total"],
        1
    );
    let stale = dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Clock(clock::Action::Inspect {
            id: evidence.monitor_pin.monitor_id,
            head: Digest32::of_bytes(b"a different clock page generation"),
            offset: 0,
            limit: 1,
        }),
    )?;
    assert_eq!(stale["result"]["ok"], false);
    assert_eq!(evidence.io.inner.connections, evidence.connections_before);
    evidence.isolated();
    Ok(())
}

#[test]
fn closed_monitor_pin_parser_and_independently_issued_opening_grants_are_minimal() -> Result<()> {
    let evidence = retained(History::Empty, 3)?;
    let pin = evidence.monitor_pin;
    let valid =
        json!({"monitor_id":pin.monitor_id.to_string(),"head":pin.head.to_string()}).to_string();
    assert_eq!(runtime::monitor_recovery(&valid)?, pin);
    let malformed = [
        "{}".to_owned(),
        json!({"monitor_id":pin.monitor_id.to_string()}).to_string(),
        json!({"monitor_id":pin.monitor_id.to_string(),"head":pin.head.to_string(),"initialize":true}).to_string(),
        json!({"monitor_id":Digest32::ZERO.to_string(),"head":pin.head.to_string()}).to_string(),
        json!({"monitor_id":pin.monitor_id.to_string(),"head":17}).to_string(),
        format!("{{\"monitor_id\":\"{}\",\"monitor_id\":\"{}\",\"head\":\"{}\"}}",pin.monitor_id,pin.monitor_id,pin.head),
        "x".repeat(2049),
    ];
    for raw in malformed {
        assert!(runtime::monitor_recovery(&raw).is_err());
    }
    for mode in [Mode::Offline, Mode::Recovery] {
        for enabled in [false, true] {
            let (c, n) = evidence.contexts(mode, enabled)?;
            let permits_clock = mode == Mode::Recovery && enabled;
            for (grants, fortress) in [
                (&c.grants, c.anchor.fortress_id),
                (&n.grants, dfmcp_core::FortressId::NIL),
            ] {
                assert!(
                    grants
                        .iter()
                        .all(|grant| grant.scope.fortress_id == Some(fortress))
                );
                assert!(
                    grants
                        .iter()
                        .any(|grant| grant.capability == Capability::Query)
                );
                assert_eq!(
                    grants
                        .iter()
                        .any(|grant| grant.capability == Capability::ControlClock),
                    permits_clock
                );
                assert!(grants.iter().all(|grant| matches!(
                    grant.capability,
                    Capability::Query | Capability::ControlClock
                )));
            }
            assert_eq!(c.budget.max_game_ticks, 0);
            assert_eq!(n.budget.max_game_ticks, 0);
        }
    }
    Ok(())
}

#[test]
fn invalidated_monitor_reopens_original_clock_with_first_accepted_operations_source() -> Result<()>
{
    let mut evidence = retained(History::MonitorSourceChanged, 3)?;
    let mut owner = evidence.open(Mode::Recovery, true)?;
    let inspected = dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Completion(completion::Action::Inspect),
    )?;
    success(&inspected);
    assert_history(&inspected, 3);
    let id = evidence.monitor_pin.monitor_id;
    success(&dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Clock(clock::Action::Wait(id)),
    )?);
    let cancelled = dispatch(
        &mut owner,
        &mut evidence.io,
        Action::Clock(clock::Action::Cancel(id)),
    )?;
    success(&cancelled);
    assert_history(&cancelled, 3);
    assert_eq!(cancelled["result"]["construction_clock"]["pending"], false);
    assert_eq!(
        evidence
            .io
            .inner
            .game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "cancel")
            .count(),
        1
    );
    evidence.isolated();
    Ok(())
}

#[test]
fn current_and_retained_clock_grants_gate_cancellation_at_the_consumed_receipt_floor() -> Result<()>
{
    for native_domain in [false, true] {
        let mut evidence = retained(History::Dispatched, 3)?;
        let mut owner = evidence.open(Mode::Recovery, true)?;
        let mut c = owner.context(None)?;
        let mut n = owner.native_context(&c)?;
        let grants = if native_domain {
            &mut n.grants
        } else {
            &mut c.grants
        };
        grants.retain(|grant| grant.capability != Capability::ControlClock);
        let rejected = decoded(owner.run_action(
            c,
            n,
            "fortress.cancel",
            Ok(Action::Clock(clock::Action::Cancel(
                evidence.monitor_pin.monitor_id,
            ))),
            Instant::now(),
            &mut evidence.io,
        ))?;
        assert_eq!(rejected["result"]["ok"], false);
        assert_eq!(evidence.io.inner.connections, evidence.connections_before);
        assert!(!evidence.io.inner.game.borrow().calls.contains(&"cancel"));
        evidence.isolated();

        let mut evidence = retained(History::Dispatched, 3)?;
        let (mut c, mut n) = evidence.contexts(Mode::Recovery, true)?;
        let grants = if native_domain {
            &mut n.grants
        } else {
            &mut c.grants
        };
        for grant in grants
            .iter_mut()
            .filter(|grant| grant.capability == Capability::ControlClock)
        {
            grant.expires_at_tick = Some(GameTick(TICK + 5));
        }
        let mut owner =
            evidence.open_with(Mode::Recovery, RunMode::Control, &c, &n, &evidence.config)?;
        evidence.io.inner.game.borrow_mut().tick = TICK + 20;
        let result = dispatch(
            &mut owner,
            &mut evidence.io,
            Action::Clock(clock::Action::Cancel(evidence.monitor_pin.monitor_id)),
        )?;
        assert_eq!(result["result"]["ok"], false);
        assert_eq!(result["result"]["effect_may_have_occurred"], true);
        assert!(owner.high_tick() >= TICK + 20);
        assert_affordance(&result, Some("fortress.query"), false);
        assert_eq!(
            evidence
                .io
                .inner
                .game
                .borrow()
                .calls
                .iter()
                .filter(|&&v| v == "cancel")
                .count(),
            1
        );
        let before = evidence.io.inner.connections;
        let (fresh_c, fresh_n) = evidence.contexts(Mode::Recovery, true)?;
        let mut c = owner.context(None)?;
        c.grants = fresh_c.grants;
        c.anchor.tick = GameTick(0);
        let mut n = owner.native_context(&c)?;
        n.grants = fresh_n.grants;
        n.anchor.tick = GameTick(0);
        let denied = decoded(owner.run_action(
            c,
            n,
            "fortress.cancel",
            Ok(Action::Clock(clock::Action::Cancel(
                evidence.monitor_pin.monitor_id,
            ))),
            Instant::now(),
            &mut evidence.io,
        ))?;
        assert_eq!(denied["result"]["ok"], false);
        assert_eq!(evidence.io.inner.connections, before);
        assert_eq!(
            evidence
                .io
                .inner
                .game
                .borrow()
                .calls
                .iter()
                .filter(|&&v| v == "cancel")
                .count(),
            1
        );
        evidence.isolated();
    }
    Ok(())
}
