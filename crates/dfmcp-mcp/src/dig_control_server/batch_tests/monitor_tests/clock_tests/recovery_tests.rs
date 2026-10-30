//! Recovery owns only the original monitor and clock, after dig custody is lost.
use super::*;
use crate::dig_control_server::clock_recovery::{Mode, RecoveryState};
use dfmcp_adapter::dig_blueprint::monitor::store::Recovery as MonitorRecovery;

#[derive(Clone, Copy)]
enum History {
    Prepared,
    Dispatched,
    SourceLost,
}

struct Retained {
    history: Memory,
    clock: Memory,
    native: Memory,
    parent: Memory,
    dig_game: Rc<RefCell<Game>>,
    native_calls: usize,
    factories: usize,
    c: OperationContext,
    config: Config,
    monitor_pin: MonitorRecovery,
    clock_pin: ClockRecovery,
    layout: Layout,
    blueprint: Digest32,
    clocks: Clocks,
}
impl Retained {
    fn contexts(&self, mode: Mode) -> Result<(OperationContext, OperationContext)> {
        let mut canonical = self.c.clone();
        canonical.grants.retain(|grant| {
            grant.capability == Capability::Query
                || (mode == Mode::Recovery && grant.capability == Capability::ControlClock)
        });
        canonical.budget.max_game_ticks = 0;
        let grants = runtime::clock_grants(mode == Mode::Recovery)
            .into_iter()
            .filter(|grant| grant.capability != Capability::Plan)
            .collect::<Vec<_>>();
        Ok((
            canonical.clone(),
            clock::native_context(&canonical, &grants)?,
        ))
    }
    fn open(&self, mode: Mode) -> Result<RecoveryState<Memory, Memory>> {
        let (canonical, native) = self.contexts(mode)?;
        self.open_config(mode, &canonical, &native, &self.config)
    }
    fn open_config(
        &self,
        mode: Mode,
        canonical: &OperationContext,
        native: &OperationContext,
        config: &Config,
    ) -> Result<RecoveryState<Memory, Memory>> {
        self.open_run_mode(mode, mode.run_mode(), canonical, native, config)
    }
    fn open_run_mode(
        &self,
        mode: Mode,
        run_mode: RunMode,
        canonical: &OperationContext,
        native: &OperationContext,
        config: &Config,
    ) -> Result<RecoveryState<Memory, Memory>> {
        let monitor = MonitorStore::open(
            self.history.clone(),
            canonical,
            DigMode::Offline,
            None,
            false,
            config.monitor.as_ref().ok_or_else(exhausted)?.recovery,
        )?;
        let journal = RunJournal::open(self.clock.clone(), native, run_mode, None, false)?;
        let owner = ClockOwner::new(
            journal,
            monitor.spec().clone(),
            config.clock.as_ref().ok_or_else(exhausted)?.recovery,
            canonical,
            native,
        )?;
        RecoveryState::new(monitor, owner, canonical, native, config, mode)
    }
    fn isolated(&self) {
        assert!(self.native.snapshot_bytes().is_empty());
        assert_eq!(self.parent.snapshot_bytes().first(), Some(&(b'D' ^ 0xff)));
        assert_eq!(self.dig_game.borrow().calls.len(), self.native_calls);
        assert_eq!(self.dig_game.borrow().factories, self.factories);
    }
}

fn retained(history: History) -> Result<Retained> {
    let (mut h, mut maps, mut clocks) = fixture()?;
    let (seal, _) = prepare(&mut h, &mut maps, &mut clocks)?;
    if !matches!(history, History::Prepared) {
        clocks.game.borrow_mut().lose_commit = true;
        let unknown = commit(&mut h, &mut maps, &mut clocks, seal)?;
        assert_eq!(unknown["result"]["ok"], false);
        assert_eq!(clocks.game.borrow().count(Call::Commit), 1);
        if matches!(history, History::SourceLost) {
            clocks.game.borrow_mut().lose_source()?;
            success(&wait(&mut h, &mut maps, &mut clocks)?);
        }
    }
    let monitor_pin = h.state.monitor.as_ref().ok_or_else(exhausted)?.recovery();
    let clock_pin = h.state.clock.as_ref().ok_or_else(exhausted)?.recovery();
    let blueprint = h.definition()?.id();
    let mut config = h.config.clone();
    config.blueprint = None;
    config.checkpoint = DigCheckpointPolicy::Required;
    let selected = config.monitor.as_mut().ok_or_else(exhausted)?;
    selected.initialize = false;
    selected.recovery = Some(monitor_pin);
    clocks.config.initialize = false;
    clocks.config.recovery = Some(clock_pin);
    config.clock = Some(clocks.config.clone());
    let mut c = context(
        SessionId::new(901),
        RequestId::new(1),
        &h.state.binding,
        0,
        h.c.budget,
        false,
    );
    c.budget.max_game_ticks = 0;
    c.grants
        .retain(|grant| grant.capability == Capability::Query);
    c.grants.push(CapabilityGrant {
        capability: Capability::ControlClock,
        scope: CapabilityScope {
            fortress_id: Some(c.anchor.fortress_id),
            ..CapabilityScope::default()
        },
        max_risk: RiskTier::Guarded,
        expires_at_tick: None,
        remaining_uses: None,
    });
    let native = h.runtime.native.clone();
    let parent = h.runtime.parent.clone();
    let dig_game = h.game.clone();
    let native_calls = dig_game.borrow().calls.len();
    let factories = dig_game.borrow().factories;
    let layout = h.layout.clone();
    let history = maps.history.clone();
    let clock = clocks.storage.clone();
    // Destroy every combined owner before the original effect inventory and
    // parent become unreadable. Recovery has no port for either source.
    drop(h);
    drop(maps);
    native.data.borrow_mut().get_mut().clear();
    parent.data.borrow_mut().get_mut()[0] ^= 0xff;
    Ok(Retained {
        history,
        clock,
        native,
        parent,
        dig_game,
        native_calls,
        factories,
        c,
        config,
        monitor_pin,
        clock_pin,
        layout,
        blueprint,
        clocks,
    })
}
fn decoded(raw: String) -> Result<Value> {
    assert!(raw.len() <= OUTPUT_BYTES as usize);
    serde_json::from_str(&raw).map_err(|_| exhausted())
}
fn dispatch_recovery(
    state: &mut RecoveryState<Memory, Memory>,
    clocks: &mut Clocks,
    op: &str,
    action: Result<Action>,
) -> Result<Value> {
    let c = state.context(None)?;
    decoded(state.run_action(c, op, action, Instant::now(), clocks))
}
fn clock_query(id: Digest32) -> Action {
    Action::Query(Query::BlueprintClock {
        monitor_id: id.to_string(),
        expected_head: None,
        offset: None,
        limit: None,
    })
}
fn monitor_query(id: Digest32) -> Action {
    Action::Query(Query::BlueprintMonitor {
        monitor_id: id.to_string(),
        expected_head: None,
        offset: None,
        limit: None,
    })
}
fn assert_scope(value: &Value) {
    assert_eq!(value["result"]["clock_recovery_only"], true);
    assert_eq!(value["result"]["native_designation_custody_opened"], false);
    assert_eq!(value["result"]["blueprint_parent_custody_opened"], false);
    assert_eq!(value["result"]["original_designations_verified"], false);
    assert_eq!(value["result"]["new_run_permitted"], false);
    assert_eq!(value["result"]["dispatch_authorized"], false);
    assert_eq!(
        value["result"]["sampled_excavation_condition_proven"],
        false
    );
    assert_eq!(clock_result(value)["fresh_goal_sampling_permitted"], false);
    assert_eq!(clock_result(value)["current_pause_proven"], false);
    assert_eq!(
        clock_result(value)["active_run"]["review_seal"],
        Value::Null
    );
    assert_no_completion(value);
}

#[test]
fn independent_offline_history_survives_lost_native_and_parent_without_source_or_writes()
-> Result<()> {
    let mut evidence = retained(History::Dispatched)?;
    let history = evidence.history.snapshot_bytes();
    let clock = evidence.clock.snapshot_bytes();
    let writes = (
        *evidence.history.writes.borrow(),
        *evidence.clock.writes.borrow(),
    );
    let syncs = (
        *evidence.history.syncs.borrow(),
        *evidence.clock.syncs.borrow(),
    );
    let connects = evidence.clocks.connects;
    let calls = evidence.clocks.game.borrow().calls.len();
    let mut state = evidence.open(Mode::Offline)?;
    assert_eq!(state.mode(), Mode::Offline);
    assert_eq!(state.high_tick(), TICK);
    let c = state.context(None)?;
    assert_eq!(c.budget.max_game_ticks, 0);
    assert!(
        c.grants
            .iter()
            .all(|grant| grant.capability == Capability::Query)
    );
    for action in [
        clock_query(evidence.monitor_pin.monitor_id),
        monitor_query(evidence.monitor_pin.monitor_id),
        Action::Inventory,
    ] {
        let value = dispatch_recovery(
            &mut state,
            &mut evidence.clocks,
            "fortress.query",
            Ok(action),
        )?;
        success(&value);
        assert_scope(&value);
    }
    let schema = dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.query",
        Ok(Action::Query(Query::Schema {})),
    )?;
    success(&schema);
    let modes = schema["result"]["query_schema"]["oneOf"]
        .as_array()
        .ok_or_else(exhausted)?
        .iter()
        .map(|row| {
            row["properties"]["mode"]["const"]
                .as_str()
                .ok_or_else(exhausted)
        })
        .collect::<Result<BTreeSet<_>>>()?;
    assert_eq!(
        modes,
        BTreeSet::from(["blueprint_clock", "blueprint_monitor", "schema"])
    );
    for action in [
        Action::WaitClock(evidence.monitor_pin.monitor_id),
        Action::CancelClock(evidence.monitor_pin.monitor_id),
    ] {
        let value = dispatch_recovery(
            &mut state,
            &mut evidence.clocks,
            "fortress.wait",
            Ok(action),
        )?;
        assert_eq!(value["result"]["ok"], false);
        assert_scope(&value);
    }
    assert_eq!(evidence.clocks.connects, connects);
    assert_eq!(evidence.clocks.game.borrow().calls.len(), calls);
    assert_eq!(evidence.history.snapshot_bytes(), history);
    assert_eq!(evidence.clock.snapshot_bytes(), clock);
    assert_eq!(
        (
            *evidence.history.writes.borrow(),
            *evidence.clock.writes.borrow()
        ),
        writes
    );
    assert_eq!(
        (
            *evidence.history.syncs.borrow(),
            *evidence.clock.syncs.borrow()
        ),
        syncs
    );
    evidence.isolated();
    Ok(())
}

#[test]
fn independent_lost_dispatch_recovery_queries_then_safety_cancels_original_once() -> Result<()> {
    let mut evidence = retained(History::Dispatched)?;
    let history = evidence.history.snapshot_bytes();
    let monitor_writes = *evidence.history.writes.borrow();
    let mut state = evidence.open(Mode::Recovery)?;
    let id = evidence.monitor_pin.monitor_id;
    let c = state.context(None)?;
    assert_eq!(c.budget.max_game_ticks, 0);
    assert!(c.grants.iter().all(|grant| matches!(
        grant.capability,
        Capability::Query | Capability::ControlClock
    )));
    let running = dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.wait",
        Ok(Action::WaitClock(id)),
    )?;
    success(&running);
    assert_scope(&running);
    assert_eq!(clock_result(&running)["unresolved"], true);
    assert_eq!(evidence.clocks.game.borrow().count(Call::Query), 1);
    let c = state.context(None)?;
    let blocked = decoded(state.close(c, false, Instant::now(), |_| Ok(())))?;
    assert_eq!(blocked["result"]["ok"], false);
    let stopped = dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.cancel",
        Ok(Action::CancelClock(id)),
    )?;
    success(&stopped);
    assert_scope(&stopped);
    assert_eq!(clock_result(&stopped)["unresolved"], false);
    assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 1);
    assert_eq!(evidence.clocks.game.borrow().count(Call::Commit), 1);
    let connects = evidence.clocks.connects;
    success(&dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.cancel",
        Ok(Action::CancelClock(id)),
    )?);
    assert_eq!(evidence.clocks.connects, connects);
    assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 1);
    let c = state.context(None)?;
    let closed = decoded(state.close(c, false, Instant::now(), |_| Ok(())))?;
    success(&closed);
    assert_eq!(closed["result"]["closed"], true);
    assert_eq!(closed["result"]["effects_cancelled"], false);
    assert_eq!(evidence.history.snapshot_bytes(), history);
    assert_eq!(*evidence.history.writes.borrow(), monitor_writes);
    evidence.isolated();
    Ok(())
}

#[test]
fn recovery_refuses_every_new_effect_and_sample_even_with_injected_broad_grants() -> Result<()> {
    let mut evidence = retained(History::Prepared)?;
    let mut state = evidence.open(Mode::Recovery)?;
    let history = evidence.history.snapshot_bytes();
    let clock = evidence.clock.snapshot_bytes();
    let id = evidence.monitor_pin.monitor_id;
    let connects = evidence.clocks.connects;
    let before = evidence.clocks.game.borrow().calls.len();
    let actions = vec![
        Action::Observe(evidence.layout.regions()[0]),
        Action::Blueprint(evidence.layout.clone()),
        Action::Plan {
            key: "replacement".into(),
            witness: id,
            hidden: false,
        },
        Action::Commit {
            key: "replacement".into(),
            plan: id,
            seal: id,
        },
        Action::Wait("original".into(), id),
        Action::Cancel("original".into(), id),
        Action::Explain("original".into(), id),
        Action::StopBlueprint(evidence.blueprint),
        Action::SampleMonitor(id),
        Action::CancelMonitor(id),
        Action::StartMonitor(monitor::Request::parse(
            &json!({"blueprint_id":evidence.blueprint.to_string(),"deadline_tick":TICK+2000})
                .to_string(),
        )?),
        Action::PrepareClock(clock::Request {
            monitor_id: id,
            game_ticks: 10,
            wall_millis: 500,
        }),
        Action::CommitClock {
            monitor_id: id,
            seal: id,
        },
        Action::Query(Query::Records {
            limit: Some(1),
            continuation: None,
        }),
        Action::Denied,
    ];
    for action in actions {
        let mut c = state.context(None)?;
        for capability in [Capability::Plan, Capability::Observe, Capability::Designate] {
            c.grants.push(CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(c.anchor.fortress_id),
                    map_area: Some(state.binding().scope()),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            });
        }
        c.budget.max_game_ticks = 1200;
        let denied = decoded(state.run_action(
            c,
            "fortress.wait",
            Ok(action),
            Instant::now(),
            &mut evidence.clocks,
        ))?;
        assert_eq!(denied["result"]["ok"], false);
        assert_scope(&denied);
    }
    assert_eq!(evidence.clocks.connects, connects);
    assert_eq!(evidence.clocks.game.borrow().calls.len(), before);
    assert_eq!(evidence.history.snapshot_bytes(), history);
    assert_eq!(evidence.clock.snapshot_bytes(), clock);
    // Retirement of a prepared original remains local and does not need Observe,
    // Plan, game-time budget, a live source or the old checkpoint exception.
    evidence.clocks.allow_control = true;
    let cancelled = dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.cancel",
        Ok(Action::CancelClock(id)),
    )?;
    success(&cancelled);
    assert_scope(&cancelled);
    assert_eq!(evidence.clocks.connects, connects);
    assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 0);
    evidence.isolated();
    Ok(())
}

#[test]
fn damaged_monitor_does_not_block_an_already_owned_original_safety_cancel() -> Result<()> {
    let mut evidence = retained(History::Dispatched)?;
    let mut state = evidence.open(Mode::Recovery)?;
    let id = evidence.monitor_pin.monitor_id;
    let monitor_writes = *evidence.history.writes.borrow();
    evidence.history.data.borrow_mut().get_mut()[0] ^= 1;
    let unreadable = dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.query",
        Ok(monitor_query(id)),
    )?;
    assert_eq!(unreadable["result"]["ok"], false);
    assert_scope(&unreadable);
    let stopped = dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.cancel",
        Ok(Action::CancelClock(id)),
    )?;
    success(&stopped);
    assert_scope(&stopped);
    assert_eq!(
        stopped["result"]["monitor_history_used_as_effect_precondition"],
        false
    );
    assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 1);
    assert_eq!(*evidence.history.writes.borrow(), monitor_writes);
    evidence.isolated();
    Ok(())
}

#[test]
fn damaged_clock_refuses_native_work_but_explicit_release_preserves_unknown_identity() -> Result<()>
{
    let mut evidence = retained(History::Dispatched)?;
    let mut state = evidence.open(Mode::Recovery)?;
    let id = evidence.monitor_pin.monitor_id;
    let connects = evidence.clocks.connects;
    let pin = state.clock_reference()["recovery"].clone();
    evidence.clock.data.borrow_mut().get_mut()[0] ^= 1;
    let rejected = dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.cancel",
        Ok(Action::CancelClock(id)),
    )?;
    assert_eq!(rejected["result"]["ok"], false);
    assert_scope(&rejected);
    assert_eq!(clock_result(&rejected)["recovery"], pin);
    assert_eq!(evidence.clocks.connects, connects);
    let mut c = state.context(None)?;
    c.grants.clear();
    let released = decoded(state.close(c, true, Instant::now(), |_| Ok(())))?;
    success(&released);
    assert_scope(&released);
    assert_eq!(released["result"]["release_for_recovery"], true);
    assert_eq!(clock_result(&released)["recovery"], pin);
    assert_eq!(clock_result(&released)["history_verified"], false);
    assert_eq!(released["result"]["effects_cancelled"], false);
    assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 0);
    evidence.isolated();
    Ok(())
}

#[test]
fn source_lost_stays_unresolved_and_requires_explicit_recovery_release() -> Result<()> {
    let mut evidence = retained(History::SourceLost)?;
    let mut state = evidence.open(Mode::Recovery)?;
    let id = evidence.monitor_pin.monitor_id;
    let connects = evidence.clocks.connects;
    for action in [Action::WaitClock(id), Action::CancelClock(id)] {
        let value = dispatch_recovery(
            &mut state,
            &mut evidence.clocks,
            "fortress.wait",
            Ok(action),
        )?;
        success(&value);
        assert_scope(&value);
        assert_eq!(clock_result(&value)["unresolved"], true);
    }
    assert_eq!(evidence.clocks.connects, connects);
    assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 0);
    let c = state.context(None)?;
    let blocked = decoded(state.close(c, false, Instant::now(), |_| Ok(())))?;
    assert_eq!(blocked["result"]["ok"], false);
    let c = state.context(None)?;
    let released = decoded(state.close(c, true, Instant::now(), |_| Ok(())))?;
    success(&released);
    assert_eq!(clock_result(&released)["unresolved"], true);
    assert_scope(&released);
    evidence.isolated();
    Ok(())
}

#[test]
fn exact_recovery_rejects_wrong_pins_sources_modes_and_manufactured_native_authority() -> Result<()>
{
    let evidence = retained(History::Prepared)?;
    let (c, n) = evidence.contexts(Mode::Recovery)?;
    let mut copied = n.clone();
    copied.grants = c.grants.clone();
    assert!(
        evidence
            .open_config(Mode::Recovery, &c, &copied, &evidence.config)
            .is_err()
    );
    let mut positive = c.clone();
    positive.budget.max_game_ticks = 1;
    assert!(
        evidence
            .open_config(Mode::Recovery, &positive, &n, &evidence.config)
            .is_err()
    );
    let mut wrong = evidence.config.clone();
    wrong.clock.as_mut().ok_or_else(exhausted)?.recovery = Some(ClockRecovery {
        journal_id: evidence.clock_pin.journal_id,
        head: Digest32::of_bytes(b"unacknowledged"),
    });
    assert!(
        evidence
            .open_config(Mode::Recovery, &c, &n, &wrong)
            .is_err()
    );
    let mut wrong = evidence.config.clone();
    wrong.monitor.as_mut().ok_or_else(exhausted)?.recovery = Some(MonitorRecovery {
        monitor_id: Digest32::of_bytes(b"different-monitor"),
        head: evidence.monitor_pin.head,
    });
    assert!(
        evidence
            .open_config(Mode::Recovery, &c, &n, &wrong)
            .is_err()
    );
    let mut wrong = evidence.config.clone();
    wrong.clock.as_mut().ok_or_else(exhausted)?.binding = RunBinding::new(
        evidence.clocks.config.binding.endpoint(),
        RunManifest {
            generation: 42,
            ..evidence.clocks.config.binding.manifest().clone()
        },
    )?;
    assert!(
        evidence
            .open_config(Mode::Recovery, &c, &n, &wrong)
            .is_err()
    );
    let mut wrong = evidence.config.clone();
    wrong.clock.as_mut().ok_or_else(exhausted)?.initialize = true;
    assert!(
        evidence
            .open_config(Mode::Recovery, &c, &n, &wrong)
            .is_err()
    );
    evidence.isolated();
    Ok(())
}

#[test]
fn consumed_recovery_receipt_clock_expires_each_independent_query_authority_without_rewind()
-> Result<()> {
    for native_expiry in [false, true] {
        let mut evidence = retained(History::Dispatched)?;
        let (mut c, mut n) = evidence.contexts(Mode::Recovery)?;
        let grants = if native_expiry {
            &mut n.grants
        } else {
            &mut c.grants
        };
        for grant in grants {
            if grant.capability == Capability::Query {
                grant.expires_at_tick = Some(GameTick(TICK + 5));
            }
        }
        let mut state = evidence.open_config(Mode::Recovery, &c, &n, &evidence.config)?;
        let learned = TICK + 20;
        evidence.clocks.game.borrow_mut().settle(learned)?;
        let id = evidence.monitor_pin.monitor_id;
        let rejected = dispatch_recovery(
            &mut state,
            &mut evidence.clocks,
            "fortress.wait",
            Ok(Action::WaitClock(id)),
        )?;
        assert_eq!(rejected["result"]["ok"], false);
        assert_scope(&rejected);
        assert!(state.high_tick() >= learned);
        assert!(
            clock_result(&rejected)["authority_tick_floor"]
                .as_u64()
                .is_some_and(|tick| tick >= learned)
        );
        assert_eq!(evidence.clocks.game.borrow().count(Call::Query), 1);
        let before = evidence.clocks.connects;
        let mut stale = state.context(None)?;
        stale.anchor.tick = GameTick(0);
        stale.budget.max_game_ticks = 1200;
        let rejected = decoded(state.run_action(
            stale,
            "fortress.cancel",
            Ok(Action::CancelClock(id)),
            Instant::now(),
            &mut evidence.clocks,
        ))?;
        assert_eq!(rejected["result"]["ok"], false);
        assert!(
            clock_result(&rejected)["authority_tick_floor"]
                .as_u64()
                .is_some_and(|tick| tick >= learned)
        );
        assert_eq!(
            rejected["agent_turn"]["budget"]["admitted"]["max_game_ticks"],
            0
        );
        assert_eq!(evidence.clocks.connects, before);
        assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 0);
        let c = state.context(None)?;
        let released = decoded(state.close(c, true, Instant::now(), |_| Ok(())))?;
        success(&released);
        assert!(
            clock_result(&released)["authority_tick_floor"]
                .as_u64()
                .is_some_and(|tick| tick >= learned)
        );
        evidence.isolated();
    }
    Ok(())
}

#[test]
fn query_only_recovery_reconciles_without_clock_authority_and_cannot_be_upgraded() -> Result<()> {
    for opening_clock_grants in [false, true] {
        let mut evidence = retained(History::Dispatched)?;
        let (mut c, mut n) = evidence.contexts(Mode::Recovery)?;
        if !opening_clock_grants {
            c.grants
                .retain(|grant| grant.capability == Capability::Query);
            n.grants
                .retain(|grant| grant.capability == Capability::Query);
        }
        let mut state =
            evidence.open_run_mode(Mode::Recovery, RunMode::Recover, &c, &n, &evidence.config)?;
        evidence.clocks.allow_control = false;
        let id = evidence.monitor_pin.monitor_id;
        let current = state.context(None)?;
        assert!(
            current
                .grants
                .iter()
                .all(|grant| grant.capability == Capability::Query)
        );
        let running = dispatch_recovery(
            &mut state,
            &mut evidence.clocks,
            "fortress.wait",
            Ok(Action::WaitClock(id)),
        )?;
        success(&running);
        assert_scope(&running);
        assert_eq!(clock_result(&running)["mode"], "recover");
        assert_eq!(evidence.clocks.game.borrow().count(Call::Query), 1);
        let connects = evidence.clocks.connects;
        let mut forged = state.context(None)?;
        forged.grants.extend(
            evidence
                .c
                .grants
                .iter()
                .filter(|grant| grant.capability == Capability::ControlClock)
                .cloned(),
        );
        evidence.clocks.allow_control = true;
        let denied = decoded(state.run_action(
            forged,
            "fortress.cancel",
            Ok(Action::CancelClock(id)),
            Instant::now(),
            &mut evidence.clocks,
        ))?;
        assert_eq!(denied["result"]["ok"], false);
        assert_scope(&denied);
        assert_eq!(evidence.clocks.connects, connects);
        assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 0);
        assert_eq!(evidence.clocks.game.borrow().count(Call::Commit), 1);
        evidence.clocks.game.borrow_mut().settle(TICK + 10)?;
        let settled = dispatch_recovery(
            &mut state,
            &mut evidence.clocks,
            "fortress.wait",
            Ok(Action::WaitClock(id)),
        )?;
        success(&settled);
        assert_scope(&settled);
        assert_eq!(clock_result(&settled)["unresolved"], false);
        evidence.isolated();
    }
    Ok(())
}

#[test]
fn cancellation_publication_rechecks_both_clock_grants_at_the_consumed_receipt_tick() -> Result<()>
{
    for native_expiry in [false, true] {
        let mut evidence = retained(History::Dispatched)?;
        let (mut c, mut n) = evidence.contexts(Mode::Recovery)?;
        let grants = if native_expiry {
            &mut n.grants
        } else {
            &mut c.grants
        };
        for grant in grants {
            if grant.capability == Capability::ControlClock {
                grant.expires_at_tick = Some(GameTick(TICK + 5));
            }
        }
        let mut state = evidence.open_config(Mode::Recovery, &c, &n, &evidence.config)?;
        let learned = TICK + 20;
        evidence.clocks.game.borrow_mut().tick = learned;
        let id = evidence.monitor_pin.monitor_id;
        let rejected = dispatch_recovery(
            &mut state,
            &mut evidence.clocks,
            "fortress.cancel",
            Ok(Action::CancelClock(id)),
        )?;
        assert_eq!(rejected["result"]["ok"], false);
        assert_scope(&rejected);
        assert_eq!(rejected["result"]["effect_may_have_occurred"], true);
        assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 1);
        assert!(state.high_tick() >= learned);
        assert!(
            clock_result(&rejected)["authority_tick_floor"]
                .as_u64()
                .is_some_and(|tick| tick >= learned)
        );
        // Query authority survives, so the completed original receipt is still
        // available without repeating the native pause or granting new time.
        let known = dispatch_recovery(
            &mut state,
            &mut evidence.clocks,
            "fortress.query",
            Ok(clock_query(id)),
        )?;
        success(&known);
        assert_scope(&known);
        assert_eq!(clock_result(&known)["unresolved"], false);
        let connects = evidence.clocks.connects;
        let denied = dispatch_recovery(
            &mut state,
            &mut evidence.clocks,
            "fortress.cancel",
            Ok(Action::CancelClock(id)),
        )?;
        assert_eq!(denied["result"]["ok"], false);
        assert_eq!(evidence.clocks.connects, connects);
        assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 1);
        evidence.isolated();
    }
    Ok(())
}

struct LateFailure<'a> {
    inner: &'a mut Clocks,
    checkpoints: usize,
    fail_at: usize,
}
impl clock::ClockIo<Memory> for LateFailure<'_> {
    type Source = ClockNative;
    fn open(
        &mut self,
        _: &MonitorSpec,
        _: &OperationContext,
        _: &OperationContext,
    ) -> Result<ClockOwner<Memory>> {
        Err(error(
            ErrorCode::InternalInvariantViolation,
            "recovery must never create a clock",
        ))
    }
    fn connect(&mut self, c: &OperationContext, control: bool) -> Result<ClockNative> {
        clock::ClockIo::connect(self.inner, c, control)
    }
    fn checkpoint(&mut self, c: &OperationContext, control: bool) -> Result<()> {
        self.checkpoints += 1;
        if self.checkpoints == self.fail_at {
            self.inner.allow_control = false;
        }
        clock::ClockIo::checkpoint(self.inner, c, control)
    }
}

#[test]
fn recovery_final_publication_rechecks_exact_custody_and_keeps_completed_safety_receipt()
-> Result<()> {
    let mut evidence = retained(History::Dispatched)?;
    let mut state = evidence.open(Mode::Recovery)?;
    let id = evidence.monitor_pin.monitor_id;
    evidence.clocks.corrupt_at_checkpoint = Some(evidence.clocks.checkpoints + 2);
    let rejected = dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.query",
        Ok(clock_query(id)),
    )?;
    assert_eq!(rejected["result"]["ok"], false);
    assert_eq!(clock_result(&rejected)["history_verified"], false);
    assert_scope(&rejected);
    let mut evidence = retained(History::Dispatched)?;
    let mut state = evidence.open(Mode::Recovery)?;
    let id = evidence.monitor_pin.monitor_id;
    let c = state.context(None)?;
    let rejected = {
        let mut io = LateFailure {
            inner: &mut evidence.clocks,
            checkpoints: 0,
            fail_at: 4,
        };
        decoded(state.run_action(
            c,
            "fortress.cancel",
            Ok(Action::CancelClock(id)),
            Instant::now(),
            &mut io,
        ))?
    };
    assert_eq!(rejected["result"]["ok"], false);
    assert_scope(&rejected);
    assert_eq!(rejected["result"]["effect_may_have_occurred"], true);
    assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 1);
    assert_eq!(clock_result(&rejected)["unresolved"], false);
    let connects = evidence.clocks.connects;
    let recovered = dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.query",
        Ok(clock_query(id)),
    )?;
    success(&recovered);
    assert_scope(&recovered);
    evidence.clocks.allow_control = true;
    success(&dispatch_recovery(
        &mut state,
        &mut evidence.clocks,
        "fortress.cancel",
        Ok(Action::CancelClock(id)),
    )?);
    assert_eq!(evidence.clocks.connects, connects);
    assert_eq!(evidence.clocks.game.borrow().count(Call::Cancel), 1);
    evidence.isolated();
    Ok(())
}
