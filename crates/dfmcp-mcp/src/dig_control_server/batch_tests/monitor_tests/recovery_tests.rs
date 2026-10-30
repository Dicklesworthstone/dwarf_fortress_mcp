//! Standalone historical recovery owns no native journal, parent or map source.
use super::*;
use crate::dig_control_server::monitor_recovery::RecoveryState;
use dfmcp_adapter::dig_blueprint::monitor::store::Recovery as MonitorRecovery;

#[derive(Clone, Copy)]
enum History {
    Registered,
    Stabilizing,
    Satisfied,
    PendingCapture,
}

struct Retained {
    history: Memory,
    native: Memory,
    parent: Memory,
    game: Rc<RefCell<Game>>,
    native_calls: usize,
    factories: usize,
    c: OperationContext,
    config: Config,
    recovery: MonitorRecovery,
    layout: Layout,
    blueprint: Digest32,
}
impl Retained {
    fn open(&self, mode: DigMode) -> Result<RecoveryState<Memory>> {
        self.open_config(mode, &self.c, &self.config)
    }
    fn open_config(
        &self,
        mode: DigMode,
        c: &OperationContext,
        config: &Config,
    ) -> Result<RecoveryState<Memory>> {
        let selected = config.monitor.as_ref().ok_or_else(exhausted)?;
        let store = MonitorStore::open(
            self.history.clone(),
            c,
            mode,
            None,
            false,
            selected.recovery,
        )?;
        RecoveryState::new(store, c, config, mode)
    }
    fn isolated(&self) {
        assert_eq!(self.game.borrow().calls.len(), self.native_calls);
        assert_eq!(self.game.borrow().factories, self.factories);
        assert!(self.native.snapshot_bytes().is_empty());
        assert_eq!(self.parent.snapshot_bytes().first(), Some(&(b'D' ^ 0xff)));
    }
}

fn retained(history: History) -> Result<Retained> {
    let (mut h, mut io) = completed(sparse_levels()?)?;
    success(&start(&mut h, &mut io)?);
    match history {
        History::Registered => {}
        History::Stabilizing => {
            let raw = capture(&h, TICK + 10)?;
            success(&sample(&mut h, &mut io, raw)?);
        }
        History::Satisfied => {
            for tick in [TICK, TICK + 10] {
                let raw = capture(&h, tick)?;
                success(&sample(&mut h, &mut io, raw)?);
            }
        }
        History::PendingCapture => {
            let raw = capture(&h, TICK + 10)?;
            let mut c = h.c.clone();
            c.request_id = RequestId::new(c.request_id.get() + 1);
            let store = h.state.monitor.as_mut().ok_or_else(exhausted)?;
            c.budget.max_bytes = store.operation_reserve();
            let attempt = store.begin_attempt(&c)?;
            store.capture(&attempt, raw, &c)?;
            assert!(store.is_fenced());
        }
    }
    let recovery = h.state.monitor.as_ref().ok_or_else(exhausted)?.recovery();
    let blueprint = h.definition()?.id();
    let mut config = h.config.clone();
    config.blueprint = None;
    let selected = config.monitor.as_mut().ok_or_else(exhausted)?;
    selected.initialize = false;
    selected.recovery = Some(recovery);
    let mut c = context(
        SessionId::new(900),
        RequestId::new(1),
        &h.state.binding,
        0,
        h.c.budget,
        false,
    );
    c.grants.retain(|g| g.capability == Capability::Query);
    let native = h.runtime.native.clone();
    let parent = h.runtime.parent.clone();
    let game = h.game.clone();
    let native_calls = game.borrow().calls.len();
    let factories = game.borrow().factories;
    let layout = h.layout.clone();
    let evidence = io.history.clone();
    // All combined owners and native preparation/source handles are gone before
    // either original custody surface is made unreadable.
    drop(h);
    drop(io);
    native.data.borrow_mut().get_mut().clear();
    parent.data.borrow_mut().get_mut()[0] ^= 0xff;
    Ok(Retained {
        history: evidence,
        native,
        parent,
        game,
        native_calls,
        factories,
        c,
        config,
        recovery,
        layout,
        blueprint,
    })
}
fn decoded(raw: String) -> Result<Value> {
    assert!(raw.len() <= OUTPUT_BYTES as usize);
    serde_json::from_str(&raw).map_err(|_| exhausted())
}
fn current(state: &mut RecoveryState<Memory>) -> Result<OperationContext> {
    state.context(None)
}
fn dispatch(state: &mut RecoveryState<Memory>, op: &str, action: Result<Action>) -> Result<Value> {
    let c = current(state)?;
    decoded(state.run_action(c, op, action, Instant::now(), |_| Ok(())))
}
fn historical(state: &mut RecoveryState<Memory>, id: Digest32) -> Result<Value> {
    dispatch(
        state,
        "fortress.query",
        Ok(Action::Query(Query::BlueprintMonitor {
            monitor_id: id.to_string(),
            expected_head: None,
            offset: None,
            limit: None,
        })),
    )
}
fn assert_historical(value: &Value) {
    success(value);
    let result = monitor_result(value);
    assert_eq!(result["history_verified"], true);
    assert_eq!(result["original_designations_verified"], false);
    assert_eq!(result["fresh_sample_acquired"], false);
    assert_eq!(result["sampled_excavation_condition_proven"], false);
    assert_eq!(result["historical_evidence_only"], true);
    assert_eq!(
        value["agent_turn"]["active_work"]["pending_absence_proven"],
        false
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["scope"],
        "this_blueprint_monitor_history_only"
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["inventory_verified"],
        false
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["bridge_protocol"],
        Value::Null
    );
    assert!(
        !value["agent_turn"]["recommendations"]
            .to_string()
            .contains("fortress.wait")
    );
}
fn assert_handoff(value: &Value, recovery: MonitorRecovery) {
    assert_eq!(value["result"]["ok"], false);
    let result = monitor_result(value);
    assert_eq!(result["monitor_id"], recovery.monitor_id.to_string());
    assert_eq!(result["recovery"], monitor::recovery(recovery));
    assert_eq!(result["history_verified"], false);
    assert_eq!(result["fresh_sample_acquired"], false);
    assert_eq!(result["sampled_excavation_condition_proven"], false);
}

#[test]
fn standalone_offline_reopens_intact_monitor_after_original_owners_and_custody_are_lost()
-> Result<()> {
    let evidence = retained(History::Stabilizing)?;
    let before = evidence.history.snapshot_bytes();
    let writes = *evidence.history.writes.borrow();
    let syncs = *evidence.history.syncs.borrow();
    let mut state = evidence.open(DigMode::Offline)?;
    assert_eq!(state.id(), evidence.c.session_id);
    assert_eq!(state.mode(), DigMode::Offline);
    assert_eq!(state.high_tick(), TICK + 10);
    let c = current(&mut state)?;
    assert_eq!(c.anchor.tick, GameTick(TICK + 10));
    assert_eq!(c.grants, evidence.c.grants);
    assert!(c.grants.iter().all(|g| g.capability == Capability::Query));
    let inspected = historical(&mut state, evidence.recovery.monitor_id)?;
    assert_historical(&inspected);
    assert_eq!(monitor_result(&inspected)["status"], "unknown");
    assert_eq!(monitor_result(&inspected)["attempts"], 1);
    assert_eq!(monitor_result(&inspected)["latest_sample_tick"], TICK + 10);
    assert_eq!(monitor_result(&inspected)["matching_samples"], 0);
    // Even an injected Observe grant cannot make this source-free profile
    // recommend the unavailable sampling route in its Agent Turn packet.
    let mut broad = c.clone();
    let mut observe = broad.grants.first().cloned().ok_or_else(exhausted)?;
    observe.capability = Capability::Observe;
    broad.grants.push(observe);
    let rendered = decoded(presentation::packet(
        "fortress.query",
        inspected["result"].clone(),
        Some(&broad),
        Some(state.binding()),
        None,
        None,
        None,
    ))?;
    assert_historical(&rendered);
    let inventory = dispatch(&mut state, "fortress.doctor", Ok(Action::Inventory))?;
    assert_historical(&inventory);
    let schema = dispatch(
        &mut state,
        "fortress.query",
        Ok(Action::Query(Query::Schema {})),
    )?;
    success(&schema);
    let alternatives = schema["result"]["query_schema"]["oneOf"]
        .as_array()
        .ok_or_else(exhausted)?;
    let modes: BTreeSet<_> = alternatives
        .iter()
        .map(|value| {
            value["properties"]["mode"]["const"]
                .as_str()
                .ok_or_else(exhausted)
        })
        .collect::<Result<_>>()?;
    assert_eq!(modes, BTreeSet::from(["blueprint_monitor", "schema"]));
    let refused = dispatch(
        &mut state,
        "fortress.cancel",
        Ok(Action::CancelMonitor(evidence.recovery.monitor_id)),
    )?;
    assert_handoff(&refused, evidence.recovery);
    assert_eq!(evidence.history.snapshot_bytes(), before);
    assert_eq!(*evidence.history.writes.borrow(), writes);
    assert_eq!(*evidence.history.syncs.borrow(), syncs);
    evidence.isolated();
    Ok(())
}

#[test]
fn standalone_recovery_stops_only_monitor_with_scoped_query_and_no_native_source() -> Result<()> {
    let evidence = retained(History::Stabilizing)?;
    let before = evidence.history.snapshot_bytes();
    let mut state = evidence.open(DigMode::Recover)?;
    let stopped = dispatch(
        &mut state,
        "fortress.cancel",
        Ok(Action::CancelMonitor(evidence.recovery.monitor_id)),
    )?;
    assert_historical(&stopped);
    assert_eq!(monitor_result(&stopped)["status"], "cancelled");
    assert_eq!(monitor_result(&stopped)["attempts"], 1);
    assert_eq!(stopped["result"]["effects_cancelled"], false);
    let head = digest(field(&monitor_result(&stopped)["recovery"]["head"])?)?;
    assert_ne!(head, evidence.recovery.head);
    assert_ne!(evidence.history.snapshot_bytes(), before);
    let after = evidence.history.snapshot_bytes();
    let replayed = dispatch(
        &mut state,
        "fortress.cancel",
        Ok(Action::CancelMonitor(evidence.recovery.monitor_id)),
    )?;
    assert_historical(&replayed);
    assert_eq!(evidence.history.snapshot_bytes(), after);
    drop(state);
    // The externally acknowledged old head may recover its complete descendant.
    let mut reopened = evidence.open(DigMode::Offline)?;
    let inspected = historical(&mut reopened, evidence.recovery.monitor_id)?;
    assert_historical(&inspected);
    assert_eq!(monitor_result(&inspected)["status"], "cancelled");
    assert_eq!(monitor_result(&inspected)["history_head"], head.to_string());
    evidence.isolated();
    Ok(())
}

#[test]
fn standalone_recovery_rejects_all_native_sampling_and_replacement_routes_before_work() -> Result<()>
{
    let evidence = retained(History::Registered)?;
    let mut state = evidence.open(DigMode::Recover)?;
    let before = evidence.history.snapshot_bytes();
    let raw = json!({"blueprint_id":evidence.blueprint.to_string(),"deadline_tick":TICK+2000})
        .to_string();
    let id = evidence.recovery.monitor_id;
    let actions = vec![
        Action::Observe(evidence.layout.regions()[0]),
        Action::Blueprint(evidence.layout.clone()),
        Action::Plan {
            key: "replacement".into(),
            witness: id,
            hidden: false,
        },
        Action::Commit {
            key: "original".into(),
            plan: id,
            seal: id,
        },
        Action::Wait("original".into(), id),
        Action::Cancel("original".into(), id),
        Action::Explain("original".into(), id),
        Action::StopBlueprint(evidence.blueprint),
        Action::SampleMonitor(id),
        Action::StartMonitor(monitor::Request::parse(&raw)?),
        Action::Query(Query::Records {
            limit: Some(1),
            continuation: None,
        }),
        Action::Query(Query::Blueprint {
            blueprint_id: evidence.blueprint.to_string(),
            expected_head: None,
            expected_parent_head: None,
            offset: None,
            limit: None,
        }),
        Action::Denied,
    ];
    for action in actions {
        let refused = dispatch(&mut state, "fortress.wait", Ok(action))?;
        assert_handoff(&refused, evidence.recovery);
        assert_eq!(evidence.history.snapshot_bytes(), before);
    }
    let inspected = historical(&mut state, id)?;
    assert_historical(&inspected);
    assert_eq!(monitor_result(&inspected)["attempts"], 0);
    assert_eq!(
        monitor_result(&inspected)["first_satisfied_tick"],
        Value::Null
    );
    evidence.isolated();
    Ok(())
}

#[test]
fn standalone_open_rejects_foreign_identity_source_scope_and_external_history_head() -> Result<()> {
    let evidence = retained(History::Registered)?;
    let before = evidence.history.snapshot_bytes();
    for mode in 0..7 {
        let mut config = evidence.config.clone();
        match mode {
            0 => {
                config.monitor.as_mut().ok_or_else(exhausted)?.recovery = Some(MonitorRecovery {
                    monitor_id: Digest32::of_bytes(b"another original monitor"),
                    head: evidence.recovery.head,
                })
            }
            1 => {
                config.monitor.as_mut().ok_or_else(exhausted)?.recovery = Some(MonitorRecovery {
                    monitor_id: evidence.recovery.monitor_id,
                    head: Digest32::of_bytes(b"unacknowledged unrelated head"),
                })
            }
            2 => {
                config.monitor.as_mut().ok_or_else(exhausted)?.source =
                    SourcePin::new(config.endpoint, 30, "test-df".into(), "test-dfhack".into())?
            }
            3 => {
                config.monitor.as_mut().ok_or_else(exhausted)?.source = SourcePin::new(
                    config.endpoint,
                    29,
                    "changed-df".into(),
                    "test-dfhack".into(),
                )?
            }
            4 => config.scope = MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(63, 63, 7))?,
            5 => config.folder = "another-world".into(),
            _ => config.monitor.as_mut().ok_or_else(exhausted)?.initialize = true,
        }
        assert!(
            evidence
                .open_config(DigMode::Recover, &evidence.c, &config)
                .is_err()
        );
        assert_eq!(evidence.history.snapshot_bytes(), before);
    }
    assert!(evidence.open(DigMode::Control).is_err());
    let mut state = evidence.open(DigMode::Recover)?;
    let wrong = Digest32::of_bytes(b"another original monitor");
    assert_handoff(&historical(&mut state, wrong)?, evidence.recovery);
    assert_handoff(
        &dispatch(
            &mut state,
            "fortress.cancel",
            Ok(Action::CancelMonitor(wrong)),
        )?,
        evidence.recovery,
    );
    let stale = dispatch(
        &mut state,
        "fortress.query",
        Ok(Action::Query(Query::BlueprintMonitor {
            monitor_id: evidence.recovery.monitor_id.to_string(),
            expected_head: Some(wrong.to_string()),
            offset: None,
            limit: None,
        })),
    )?;
    assert_handoff(&stale, evidence.recovery);
    assert_eq!(evidence.history.snapshot_bytes(), before);
    evidence.isolated();
    Ok(())
}

#[test]
fn standalone_clock_floor_does_not_restore_expired_or_removed_read_authority() -> Result<()> {
    let evidence = retained(History::Stabilizing)?;
    let mut expired = evidence.c.clone();
    expired.anchor.tick = GameTick(0);
    expired
        .grants
        .iter_mut()
        .for_each(|g| g.expires_at_tick = Some(GameTick(TICK + 9)));
    assert!(
        evidence
            .open_config(DigMode::Recover, &expired, &evidence.config)
            .is_err()
    );
    let mut state = evidence.open(DigMode::Recover)?;
    let mut c = current(&mut state)?;
    assert_eq!(c.anchor.tick, GameTick(TICK + 10));
    c.anchor.tick = GameTick(0);
    c.grants = expired.grants.clone();
    let refused = decoded(state.run_action(
        c,
        "fortress.query",
        Ok(Action::Inventory),
        Instant::now(),
        |_| Ok(()),
    ))?;
    assert_handoff(&refused, evidence.recovery);
    let mut c = current(&mut state)?;
    c.grants.clear();
    let refused = decoded(state.run_action(
        c,
        "fortress.cancel",
        Ok(Action::CancelMonitor(evidence.recovery.monitor_id)),
        Instant::now(),
        |_| Ok(()),
    ))?;
    assert_handoff(&refused, evidence.recovery);
    let inspected = historical(&mut state, evidence.recovery.monitor_id)?;
    assert_historical(&inspected);
    assert_eq!(monitor_result(&inspected)["attempts"], 1);
    assert_eq!(
        monitor_result(&inspected)["authority_tick_floor"],
        TICK + 10
    );
    drop(state);
    let mut expiring = evidence.c.clone();
    expiring.grants.iter_mut().for_each(|grant| {
        grant.expires_at_tick = Some(GameTick(TICK + 15));
    });
    let mut state = evidence.open_config(DigMode::Recover, &expiring, &evidence.config)?;
    let mut forged = current(&mut state)?;
    forged.anchor.tick = GameTick(TICK + 20);
    forged
        .grants
        .iter_mut()
        .for_each(|grant| grant.expires_at_tick = None);
    let denied = decoded(state.run_action(
        forged,
        "fortress.query",
        Ok(Action::Inventory),
        Instant::now(),
        |_| Ok(()),
    ))?;
    assert_handoff(&denied, evidence.recovery);
    evidence.isolated();
    Ok(())
}

#[test]
fn standalone_terminal_satisfaction_is_historical_and_never_enables_sampling() -> Result<()> {
    let evidence = retained(History::Satisfied)?;
    let before = evidence.history.snapshot_bytes();
    let mut state = evidence.open(DigMode::Recover)?;
    let inspected = historical(&mut state, evidence.recovery.monitor_id)?;
    assert_historical(&inspected);
    assert_eq!(monitor_result(&inspected)["status"], "satisfied");
    assert_eq!(
        monitor_result(&inspected)["historical_satisfaction_recorded"],
        true
    );
    assert_eq!(
        monitor_result(&inspected)["first_satisfied_tick"],
        TICK + 10
    );
    let denied = dispatch(
        &mut state,
        "fortress.wait",
        Ok(Action::SampleMonitor(evidence.recovery.monitor_id)),
    )?;
    assert_handoff(&denied, evidence.recovery);
    let stopped = dispatch(
        &mut state,
        "fortress.cancel",
        Ok(Action::CancelMonitor(evidence.recovery.monitor_id)),
    )?;
    assert_historical(&stopped);
    assert_eq!(monitor_result(&stopped)["status"], "satisfied");
    assert_eq!(evidence.history.snapshot_bytes(), before);
    evidence.isolated();
    Ok(())
}

#[test]
fn standalone_output_and_runtime_failures_keep_exact_recovery_including_completed_stop()
-> Result<()> {
    let evidence = retained(History::Registered)?;
    let mut state = evidence.open(DigMode::Recover)?;
    let mut tiny = current(&mut state)?;
    tiny.budget.max_output_tokens = 1;
    let failed = decoded(state.run_action(
        tiny,
        "fortress.query",
        Ok(Action::Inventory),
        Instant::now(),
        |_| Ok(()),
    ))?;
    assert_handoff(&failed, evidence.recovery);
    let c = current(&mut state)?;
    let failed = decoded(state.run_action(
        c,
        "fortress.query",
        Ok(Action::Inventory),
        Instant::now(),
        |_| {
            Err(error(
                ErrorCode::CancellationRequested,
                "runtime stopped before inspection",
            ))
        },
    ))?;
    assert_handoff(&failed, evidence.recovery);
    let before = evidence.history.snapshot_bytes();
    let memory = evidence.history.clone();
    let c = current(&mut state)?;
    let failed = decoded(state.run_action(
        c,
        "fortress.cancel",
        Ok(Action::CancelMonitor(evidence.recovery.monitor_id)),
        Instant::now(),
        |_| {
            if memory.snapshot_bytes() != before {
                Err(error(
                    ErrorCode::CancellationRequested,
                    "runtime stopped after durable local stop",
                ))
            } else {
                Ok(())
            }
        },
    ))?;
    assert_eq!(failed["result"]["ok"], false);
    let recovery = MonitorRecovery {
        monitor_id: evidence.recovery.monitor_id,
        head: digest(field(&monitor_result(&failed)["recovery"]["head"])?)?,
    };
    assert_ne!(recovery.head, evidence.recovery.head);
    assert_handoff(&failed, recovery);
    let inspected = historical(&mut state, evidence.recovery.monitor_id)?;
    assert_historical(&inspected);
    assert_eq!(monitor_result(&inspected)["status"], "cancelled");
    assert_eq!(
        monitor_result(&inspected)["history_head"],
        recovery.head.to_string()
    );
    evidence.isolated();
    Ok(())
}

#[test]
fn standalone_close_preserves_fenced_read_and_explicit_release_survives_corrupt_history()
-> Result<()> {
    let evidence = retained(History::PendingCapture)?;
    let before = evidence.history.snapshot_bytes();
    let mut state = evidence.open(DigMode::Recover)?;
    let inspected = historical(&mut state, evidence.recovery.monitor_id)?;
    assert_historical(&inspected);
    assert_eq!(monitor_result(&inspected)["sampling_fenced"], true);
    assert_eq!(monitor_result(&inspected)["attempts"], 1);
    let c = current(&mut state)?;
    let closed = decoded(state.close(c, false, Instant::now(), |_| Ok(())))?;
    success(&closed);
    assert_eq!(closed["result"]["closed"], true);
    assert_eq!(closed["result"]["effects_cancelled"], false);
    assert_eq!(
        closed["agent_turn"]["active_work"]["pending_absence_proven"],
        false
    );
    assert_eq!(evidence.history.snapshot_bytes(), before);
    drop(state);
    let mut state = evidence.open(DigMode::Recover)?;
    let inspected = historical(&mut state, evidence.recovery.monitor_id)?;
    assert_historical(&inspected);
    assert_eq!(monitor_result(&inspected)["sampling_fenced"], true);
    evidence.history.data.borrow_mut().get_mut()[0] ^= 1;
    let corrupted = evidence.history.snapshot_bytes();
    let c = current(&mut state)?;
    let ordinary = decoded(state.close(c, false, Instant::now(), |_| Ok(())))?;
    assert_handoff(&ordinary, evidence.recovery);
    let mut wrong = current(&mut state)?;
    wrong.session_id = SessionId::new(wrong.session_id.get() + 1);
    let denied = decoded(state.close(wrong, true, Instant::now(), |_| Ok(())))?;
    assert_handoff(&denied, evidence.recovery);
    let mut c = current(&mut state)?;
    c.grants.clear();
    let released = decoded(state.close(c, true, Instant::now(), |_| Ok(())))?;
    success(&released);
    assert_eq!(released["result"]["closed"], true);
    assert_eq!(released["result"]["release_for_recovery"], true);
    assert_eq!(
        monitor_result(&released)["recovery"],
        monitor::recovery(evidence.recovery)
    );
    assert_eq!(monitor_result(&released)["history_verified"], false);
    assert_eq!(
        monitor_result(&released)["sampled_excavation_condition_proven"],
        false
    );
    assert_eq!(evidence.history.snapshot_bytes(), corrupted);
    evidence.isolated();
    Ok(())
}

#[test]
fn monitor_only_opening_modes_cannot_fall_back_to_native_bootstrap_or_initialization() -> Result<()>
{
    let evidence = retained(History::Registered)?;
    for (name, mode) in [
        ("monitor-offline", DigMode::Offline),
        ("monitor-recovery", DigMode::Recover),
    ] {
        let (actual, region, only) = opening_selection(Some(name), None, &evidence.config)?;
        assert_eq!(actual, mode);
        assert!(region.is_none());
        assert!(only);
        assert!(opening_selection(Some(name), Some("[2,2,2,1,1]"), &evidence.config).is_err());
        for change in 0..3 {
            let mut config = evidence.config.clone();
            match change {
                0 => config.monitor = None,
                1 => config.monitor.as_mut().ok_or_else(exhausted)?.initialize = true,
                _ => config.monitor.as_mut().ok_or_else(exhausted)?.recovery = None,
            }
            assert!(opening_selection(Some(name), None, &config).is_err());
        }
    }
    for mode in [
        None,
        Some("control"),
        Some("offline"),
        Some("reconcile"),
        Some("monitor"),
        Some("recovery"),
    ] {
        assert!(opening_selection(mode, None, &evidence.config).is_err());
        assert!(opening_selection(mode, Some("[2,2,2,1,1]"), &evidence.config).is_err());
    }
    evidence.isolated();
    Ok(())
}
