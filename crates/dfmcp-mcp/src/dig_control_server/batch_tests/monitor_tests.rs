//! Complete handler monitoring after real multi-region designation receipts.
//! Map terrain is independent evidence; fixture designation never changes it.
use super::*;
use dfmcp_adapter::dig_blueprint::monitor::store::MonitorStore;
use dfmcp_adapter::dig_blueprint::monitor::{MonitorSpec, SourcePin, Status};
use dfmcp_adapter::live_map::LiveMapObservation;
use dfmcp_world::map_region::{Cell, MapRegion, Shape, Tile};
use std::collections::VecDeque;

struct Maps {
    history: Memory,
    captures: VecDeque<LiveMapObservation>,
    creations: usize,
    reads: usize,
    mode: DigMode,
    fail_after_read: Option<ErrorCode>,
    boundary_failure: Option<ErrorCode>,
}
impl Default for Maps {
    fn default() -> Self {
        Self {
            history: Memory::default(),
            captures: VecDeque::new(),
            creations: 0,
            reads: 0,
            mode: DigMode::Control,
            fail_after_read: None,
            boundary_failure: None,
        }
    }
}
impl monitor::MonitorIo<Memory> for Maps {
    fn create_history(
        &mut self,
        spec: MonitorSpec,
        c: &OperationContext,
    ) -> Result<MonitorStore<Memory>> {
        self.creations += 1;
        assert!(self.history.snapshot_bytes().is_empty());
        MonitorStore::open(self.history.clone(), c, self.mode, Some(spec), true, None)
    }
    fn read_map(&mut self, spec: &MonitorSpec, c: &OperationContext) -> Result<LiveMapObservation> {
        dfmcp_adapter::dig_blueprint::monitor::io::preflight(spec, c)?;
        self.reads += 1;
        self.boundary_failure = self.fail_after_read.take();
        let value = self
            .captures
            .pop_front()
            .ok_or_else(|| error(ErrorCode::AdapterUnavailable, "no mock map capture queued"))?;
        // Exercise the real typed wire decoder, including complete cell count.
        LiveMapObservation::decode_payload(
            &value.encode_payload()?,
            value.bridge_generation,
            value.df_version,
            value.dfhack_version,
        )
    }
    fn checkpoint(&mut self, c: &OperationContext) -> Result<()> {
        c.budget.validate()?;
        if c.cancellation_requested {
            return Err(error(
                ErrorCode::CancellationRequested,
                "test request cancelled",
            ));
        }
        if let Some(code) = self.boundary_failure.take() {
            return Err(error(code, "runtime revoked after complete map capture"));
        }
        Ok(())
    }
}

fn sparse_levels() -> Result<Layout> {
    Layout::new(vec![
        BlueprintPart {
            origin: [2, 2, 2],
            size: [9, 1, 1],
        },
        BlueprintPart {
            origin: [2, 4, 3],
            size: [2, 1, 1],
        },
    ])
}
fn configure(h: &mut Harness) -> Result<Maps> {
    let value = monitor::Config {
        path: "/private/dig/monitor".into(),
        initialize: true,
        source: SourcePin::new(
            h.config.endpoint,
            29,
            "test-df".into(),
            "test-dfhack".into(),
        )?,
        recovery: None,
    };
    h.config.monitor = Some(value.clone());
    h.state.monitor_config = Some(value);
    h.c.budget.max_entities = monitor::MAX_ENTITIES;
    h.state.budget = h.c.budget;
    Ok(Maps::default())
}
fn completed(layout: Layout) -> Result<(Harness, Maps)> {
    let mut h = Harness::new(layout, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    for index in 0..h.layout.regions().len() {
        let (plan, seal, _) = h.prepare(index)?;
        success(&h.commit(&plan, seal)?);
    }
    assert_eq!(
        h.game.borrow().targets,
        h.layout.targets().iter().copied().collect()
    );
    let io = configure(&mut h)?;
    Ok((h, io))
}
fn run(h: &mut Harness, io: &mut Maps, op: &str, action: Result<Action>) -> Result<Value> {
    h.c.request_id = RequestId::new(h.c.request_id.get() + 1);
    h.c.anchor.tick = GameTick(
        h.c.anchor
            .tick
            .get()
            .max(h.state.authority_tick)
            .max(h.state.control.high_tick()),
    );
    let game = h.game.clone();
    let raw = run_action_with_monitor(
        &mut h.state,
        h.c.clone(),
        op,
        action,
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
        io,
    );
    assert!(
        raw.len() <= OUTPUT_BYTES as usize,
        "monitor response exceeded reserved output"
    );
    serde_json::from_str(&raw).map_err(|_| exhausted())
}
fn request(h: &Harness, deadline: u64) -> Result<Action> {
    monitor::Request::parse(
        &json!({"blueprint_id":h.definition()?.id().to_string(),
        "deadline_tick":deadline})
        .to_string(),
    )
    .map(Action::StartMonitor)
}
fn start(h: &mut Harness, io: &mut Maps) -> Result<Value> {
    let action = request(h, TICK + 1000);
    run(h, io, "fortress.plan", action)
}
fn spec(h: &Harness) -> Result<&MonitorSpec> {
    h.state
        .monitor
        .as_ref()
        .map(MonitorStore::spec)
        .ok_or_else(exhausted)
}
fn floor() -> Cell {
    visible(Shape::Floor)
}
fn visible(shape: Shape) -> Cell {
    Cell::Visible(Tile {
        native_tiletype: 200,
        shape,
        liquid_depth: 0,
        magma: false,
        traffic: 0,
        dig_designation: 0,
        building_occupancy: 0,
        unit_occupancy: 0,
        walkable_region: 1,
        temperature_1: 10000,
        temperature_2: 10000,
    })
}
fn capture(h: &Harness, tick: u64) -> Result<LiveMapObservation> {
    let spec = spec(h)?;
    let region = spec.region();
    let mut cells = vec![Cell::Hidden; region.volume().map_err(|_| exhausted())?];
    for point in spec.parent().layout().targets() {
        cells[region.index(*point).ok_or_else(exhausted)?] = floor();
    }
    let value = LiveMapObservation {
        bridge_generation: spec.source().generation(),
        df_version: spec.source().df_version().to_owned(),
        dfhack_version: spec.source().dfhack_version().to_owned(),
        year: u32::try_from(tick / 403200).map_err(|_| exhausted())?,
        year_tick: (tick % 403200) as u32,
        paused: true,
        site_id: spec.parent().binding().site(),
        world_folder: spec.parent().binding().folder().to_owned(),
        map_dimensions: spec.parent().bootstrap().dimensions(),
        map: MapRegion { region, cells },
    };
    value.validate()?;
    Ok(value)
}
fn replace(raw: &mut LiveMapObservation, point: [u32; 3], cell: Cell) -> Result<()> {
    let index = raw.map.region.index(point).ok_or_else(exhausted)?;
    raw.map.cells[index] = cell;
    Ok(())
}
fn wall() -> Cell {
    visible(Shape::Wall)
}
fn sample(h: &mut Harness, io: &mut Maps, raw: LiveMapObservation) -> Result<Value> {
    let id = spec(h)?.id();
    io.captures.push_back(raw);
    run(h, io, "fortress.wait", Ok(Action::SampleMonitor(id)))
}
fn query(h: &mut Harness, io: &mut Maps) -> Result<Value> {
    let id = spec(h)?.id();
    run(
        h,
        io,
        "fortress.query",
        Ok(Action::Query(Query::BlueprintMonitor {
            monitor_id: id.to_string(),
            expected_head: None,
            offset: None,
            limit: None,
        })),
    )
}
fn reopen_history(h: &mut Harness, io: &mut Maps, mode: DigMode) -> Result<()> {
    let retained = h.state.monitor.take().ok_or_else(exhausted)?;
    let recovery = retained.recovery();
    drop(retained);
    let store = MonitorStore::open(io.history.clone(), &h.c, mode, None, false, Some(recovery))?;
    h.state.authority_tick = h.state.authority_tick.max(store.high_tick());
    h.state.monitor_recovery = Some(store.recovery());
    let mut config = h.state.monitor_config.clone().ok_or_else(exhausted)?;
    config.initialize = false;
    config.recovery = Some(recovery);
    h.config.monitor = Some(config.clone());
    h.state.monitor_config = Some(config);
    h.state.monitor = Some(store);
    Ok(())
}
fn monitor_result(value: &Value) -> &Value {
    &value["result"]["blueprint_monitor"]
}

#[test]
fn monitor_proves_whole_sparse_multilevel_mask_without_aggregating_parts_or_equal_ticks()
-> Result<()> {
    let (mut h, mut io) = completed(sparse_levels()?)?;
    assert_eq!(h.layout.regions().len(), 3);
    let native = h.runtime.native.snapshot_bytes();
    let parent = h.runtime.parent.snapshot_bytes();
    let contacts = h.game.borrow().calls.len();
    success(&start(&mut h, &mut io)?);
    assert_eq!(spec(&h)?.region().size, [9, 3, 2]);
    let original = spec(&h)?.id();
    let mut first = capture(&h, TICK)?;
    replace(&mut first, [2, 2, 2], wall())?;
    let first = sample(&mut h, &mut io, first)?;
    success(&first);
    assert_eq!(monitor_result(&first)["counts"]["matched"], 10);
    assert_eq!(monitor_result(&first)["counts"]["hidden"], 0);
    let mut alternate = capture(&h, TICK + 10)?;
    replace(&mut alternate, [2, 4, 3], wall())?;
    let alternate = sample(&mut h, &mut io, alternate)?;
    success(&alternate);
    assert_eq!(monitor_result(&alternate)["status"], "pending");
    assert_eq!(monitor_result(&alternate)["matching_samples"], 0);
    assert_eq!(
        monitor_result(&alternate)["sampled_excavation_condition_proven"],
        false
    );
    for tick in [TICK + 20, TICK + 20] {
        let raw = capture(&h, tick)?;
        let value = sample(&mut h, &mut io, raw)?;
        success(&value);
        assert_eq!(monitor_result(&value)["status"], "stabilizing");
        assert_eq!(monitor_result(&value)["matching_samples"], 1);
    }
    let raw = capture(&h, TICK + 30)?;
    let proven = sample(&mut h, &mut io, raw)?;
    success(&proven);
    let proof = monitor_result(&proven);
    assert_eq!(proof["status"], "satisfied");
    assert_eq!(proof["counts"]["matched"], 11);
    assert_eq!(proof["attempts"], 5);
    assert_eq!(proof["sampled_excavation_condition_proven"], true);
    assert_eq!(proof["original_designations_verified"], true);
    assert_eq!(proof["fresh_sample_acquired"], true);
    assert_eq!(proof["shared_process_identity_proven"], false);
    assert_eq!(proof["mining_causality_proven"], false);
    assert_eq!(proof["game_ticks_advanced"], 0);
    assert_eq!(proof["source"]["generation"], 29);
    assert_eq!(proof["source"]["dig_generation"], 7);
    assert_eq!(spec(&h)?.id(), original);
    assert_eq!(h.game.borrow().calls.len(), contacts);
    assert_eq!(h.runtime.native.snapshot_bytes(), native);
    assert_eq!(h.runtime.parent.snapshot_bytes(), parent);
    let historical = query(&mut h, &mut io)?;
    success(&historical);
    assert_eq!(monitor_result(&historical)["status"], "satisfied");
    assert_eq!(monitor_result(&historical)["fresh_sample_acquired"], false);
    assert_eq!(
        monitor_result(&historical)["sampled_excavation_condition_proven"],
        false
    );
    assert_eq!(
        monitor_result(&historical)["historical_satisfaction_recorded"],
        true
    );
    reopen_history(&mut h, &mut io, DigMode::Recover)?;
    let reopened = query(&mut h, &mut io)?;
    success(&reopened);
    assert_eq!(monitor_result(&reopened)["historical_evidence_only"], true);
    assert_eq!(monitor_result(&reopened)["first_satisfied_tick"], TICK + 30);
    let replay = run(
        &mut h,
        &mut io,
        "fortress.wait",
        Ok(Action::SampleMonitor(original)),
    )?;
    success(&replay);
    assert_eq!(replay["result"]["replayed_locally"], true);
    assert_eq!(
        monitor_result(&replay)["sampled_excavation_condition_proven"],
        false
    );
    assert_eq!(io.reads, 5);
    Ok(())
}

#[test]
fn monitor_registration_requires_every_original_terminal_designation() -> Result<()> {
    let mut h = Harness::new(sparse_levels()?, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    let mut io = configure(&mut h)?;
    let before = h.runtime.native.snapshot_bytes();
    assert_eq!(start(&mut h, &mut io)?["result"]["ok"], false);
    assert_eq!(h.runtime.native.snapshot_bytes(), before);
    let (plan, _, _) = h.prepare(0)?;
    let contacts = h.game.borrow().calls.len();
    assert_eq!(start(&mut h, &mut io)?["result"]["ok"], false);
    assert!(h.state.review.is_none());
    assert!(!h.state.control.has_preparation_connection());
    assert_eq!(
        h.state
            .control
            .get(plan.key(), plan.digest(), &h.c)?
            .state(),
        DigState::Prepared
    );
    assert_eq!(h.game.borrow().calls.len(), contacts);
    assert_eq!(io.creations, 0);
    assert!(io.history.snapshot_bytes().is_empty());

    let mut h = Harness::new(sparse_levels()?, DigCheckpointPolicy::DisposableFortress)?;
    success(&h.publish()?);
    for index in 0..h.layout.regions().len() {
        let (plan, seal, _) = h.prepare(index)?;
        if index + 1 == h.layout.regions().len() {
            h.game.borrow_mut().reply = Reply::Unknown;
            let _ = h.commit(&plan, seal)?;
            assert_eq!(
                h.state
                    .control
                    .get(plan.key(), plan.digest(), &h.c)?
                    .effect()
                    .map(DigEffect::phase),
                Some(DigPhase::Unknown)
            );
        } else {
            success(&h.commit(&plan, seal)?);
        }
    }
    let mut io = configure(&mut h)?;
    let contacts = h.game.borrow().calls.len();
    assert_eq!(start(&mut h, &mut io)?["result"]["ok"], false);
    assert_eq!(io.creations, 0);
    assert_eq!(io.reads, 0);
    assert_eq!(h.game.borrow().calls.len(), contacts);
    Ok(())
}

#[test]
fn stopped_completed_original_can_be_monitored_with_recover_read_authority() -> Result<()> {
    let (mut h, _) = completed(sparse_levels()?)?;
    let id = h.definition()?.id();
    success(&h.run("fortress.cancel", Ok(Action::StopBlueprint(id)))?);
    h.reopen(DigMode::Recover)?;
    let mut io = configure(&mut h)?;
    io.mode = DigMode::Recover;
    h.c.grants
        .retain(|g| matches!(g.capability, Capability::Query | Capability::Observe));
    assert!(
        h.c.grants
            .iter()
            .all(|g| g.scope.map_area == Some(h.config.scope))
    );
    let contacts = h.game.borrow().calls.len();
    let native = h.runtime.native.snapshot_bytes();
    let parent = h.runtime.parent.snapshot_bytes();
    success(&start(&mut h, &mut io)?);
    assert!(h.state.blueprint.as_ref().ok_or_else(exhausted)?.stopped());
    for tick in [TICK, TICK + 10] {
        let raw = capture(&h, tick)?;
        success(&sample(&mut h, &mut io, raw)?);
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
    assert_eq!(h.game.borrow().calls.len(), contacts);
    assert_eq!(h.runtime.native.snapshot_bytes(), native);
    assert_eq!(h.runtime.parent.snapshot_bytes(), parent);
    Ok(())
}

#[test]
fn monitor_query_and_local_stop_survive_damaged_parent_and_native_custody() -> Result<()> {
    let (mut h, mut io) = completed(sparse_levels()?)?;
    success(&start(&mut h, &mut io)?);
    let raw = capture(&h, TICK)?;
    success(&sample(&mut h, &mut io, raw)?);
    let id = spec(&h)?.id();
    let contacts = h.game.borrow().calls.len();
    h.runtime.native.data.borrow_mut().get_mut()[0] ^= 1;
    h.runtime.parent.data.borrow_mut().get_mut()[0] ^= 1;
    let native = h.runtime.native.snapshot_bytes();
    let parent = h.runtime.parent.snapshot_bytes();
    let inspected = query(&mut h, &mut io)?;
    success(&inspected);
    assert_eq!(
        monitor_result(&inspected)["original_designations_verified"],
        false
    );
    assert_eq!(monitor_result(&inspected)["fresh_sample_acquired"], false);
    let refused = run(
        &mut h,
        &mut io,
        "fortress.wait",
        Ok(Action::SampleMonitor(id)),
    )?;
    assert_eq!(refused["result"]["ok"], false);
    assert_eq!(io.reads, 1);
    h.c.grants.retain(|g| g.capability == Capability::Query);
    let stopped = run(
        &mut h,
        &mut io,
        "fortress.cancel",
        Ok(Action::CancelMonitor(id)),
    )?;
    success(&stopped);
    assert_eq!(monitor_result(&stopped)["status"], "cancelled");
    assert_eq!(stopped["result"]["effects_cancelled"], false);
    assert_eq!(h.game.borrow().calls.len(), contacts);
    assert_eq!(h.runtime.native.snapshot_bytes(), native);
    assert_eq!(h.runtime.parent.snapshot_bytes(), parent);
    assert_eq!(io.reads, 1);
    Ok(())
}

#[test]
fn map_source_drift_is_durable_and_preserves_last_accepted_original_sample() -> Result<()> {
    let (mut h, mut io) = completed(sparse_levels()?)?;
    success(&start(&mut h, &mut io)?);
    let raw = capture(&h, TICK)?;
    success(&sample(&mut h, &mut io, raw)?);
    let mut changed = capture(&h, TICK + 10)?;
    changed.bridge_generation += 1;
    let invalidated = sample(&mut h, &mut io, changed)?;
    success(&invalidated);
    let result = monitor_result(&invalidated);
    assert_eq!(result["status"], "invalidated");
    assert_eq!(result["latest_sample_tick"], TICK);
    assert_eq!(result["authority_tick_floor"], TICK + 10);
    assert_eq!(result["sampled_excavation_condition_proven"], false);
    assert_eq!(result["matching_samples"], 0);
    reopen_history(&mut h, &mut io, DigMode::Recover)?;
    let inspected = query(&mut h, &mut io)?;
    success(&inspected);
    assert_eq!(monitor_result(&inspected)["status"], "invalidated");
    assert_eq!(
        monitor_result(&inspected)["authority_tick_floor"],
        TICK + 10
    );
    let id = spec(&h)?.id();
    let replay = run(
        &mut h,
        &mut io,
        "fortress.wait",
        Ok(Action::SampleMonitor(id)),
    )?;
    success(&replay);
    assert_eq!(io.reads, 2);
    Ok(())
}

#[test]
fn captured_new_clock_expires_authority_without_publishing_success_or_rewinding() -> Result<()> {
    let (mut h, mut io) = completed(sparse_levels()?)?;
    success(&start(&mut h, &mut io)?);
    h.c.grants
        .iter_mut()
        .for_each(|g| g.expires_at_tick = Some(GameTick(TICK + 5)));
    let grants = h.c.grants.clone();
    let raw = capture(&h, TICK + 10)?;
    let failed = sample(&mut h, &mut io, raw)?;
    assert_eq!(failed["result"]["ok"], false);
    assert_eq!(io.reads, 1);
    assert_eq!(h.c.grants, grants);
    let store = h.state.monitor.as_ref().ok_or_else(exhausted)?;
    assert_eq!(store.high_tick(), TICK + 10);
    assert_eq!(store.state().attempts(), 1);
    assert!(store.state().first_satisfied().is_none());
    assert_eq!(h.state.authority_tick, TICK + 10);
    assert_eq!(query(&mut h, &mut io)?["result"]["ok"], false);
    // New host authority can inspect retained negative custody, but cannot
    // turn the interrupted raw sample into a successful completed attempt.
    h.c.grants.iter_mut().for_each(|g| g.expires_at_tick = None);
    reopen_history(&mut h, &mut io, DigMode::Recover)?;
    let inspected = query(&mut h, &mut io)?;
    success(&inspected);
    assert_eq!(monitor_result(&inspected)["sampling_fenced"], true);
    assert_eq!(
        monitor_result(&inspected)["sampled_excavation_condition_proven"],
        false
    );
    assert_eq!(
        monitor_result(&inspected)["authority_tick_floor"],
        TICK + 10
    );
    let id = spec(&h)?.id();
    assert_eq!(
        run(
            &mut h,
            &mut io,
            "fortress.wait",
            Ok(Action::SampleMonitor(id))
        )?["result"]["ok"],
        false
    );
    assert_eq!(io.reads, 1);
    success(&run(
        &mut h,
        &mut io,
        "fortress.cancel",
        Ok(Action::CancelMonitor(id)),
    )?);
    Ok(())
}

#[test]
fn failed_runtime_publication_consumes_attempt_and_breaks_stability() -> Result<()> {
    let (mut h, mut io) = completed(sparse_levels()?)?;
    success(&start(&mut h, &mut io)?);
    io.fail_after_read = Some(ErrorCode::CancellationRequested);
    let raw = capture(&h, TICK)?;
    let failed = sample(&mut h, &mut io, raw)?;
    assert_eq!(failed["result"]["ok"], false);
    let inspected = query(&mut h, &mut io)?;
    success(&inspected);
    assert_eq!(monitor_result(&inspected)["status"], "unknown");
    assert_eq!(monitor_result(&inspected)["matching_samples"], 0);
    assert_eq!(
        monitor_result(&inspected)["latest_sample_tick"],
        Value::Null
    );
    let raw = capture(&h, TICK + 10)?;
    let first = sample(&mut h, &mut io, raw)?;
    success(&first);
    assert_eq!(monitor_result(&first)["matching_samples"], 1);
    assert_eq!(
        monitor_result(&first)["sampled_excavation_condition_proven"],
        false
    );
    let raw = capture(&h, TICK + 20)?;
    let second = sample(&mut h, &mut io, raw)?;
    success(&second);
    assert_eq!(monitor_result(&second)["status"], "satisfied");
    assert_eq!(monitor_result(&second)["attempts"], 3);
    assert_eq!(io.reads, 3);
    Ok(())
}

#[test]
fn monitor_budget_and_authority_rejection_happen_before_history_or_map_io() -> Result<()> {
    let (mut h, mut io) = completed(sparse_levels()?)?;
    let budget = h.c.budget;
    let contacts = h.game.borrow().calls.len();
    h.c.budget.max_bytes = OUTPUT_BYTES + 2 * VIEW_BYTES + 1;
    assert_eq!(start(&mut h, &mut io)?["result"]["ok"], false);
    assert_eq!(io.creations, 0);
    assert!(io.history.snapshot_bytes().is_empty());
    h.c.budget = budget;
    success(&start(&mut h, &mut io)?);
    let before = io.history.snapshot_bytes();
    let id = spec(&h)?.id();
    let grants = h.c.grants.clone();
    for mode in 0..4 {
        h.c.budget = budget;
        h.c.grants = grants.clone();
        h.c.cancellation_requested = false;
        match mode {
            0 => h.c.budget.max_bytes = OUTPUT_BYTES + 2 * VIEW_BYTES + 1,
            1 => h.c.budget.max_entities = 1,
            2 => h.c.grants.retain(|g| g.capability != Capability::Observe),
            _ => h.c.cancellation_requested = true,
        }
        let denied = run(
            &mut h,
            &mut io,
            "fortress.wait",
            Ok(Action::SampleMonitor(id)),
        )?;
        assert_eq!(denied["result"]["ok"], false);
        assert_eq!(io.reads, 0);
        assert_eq!(io.history.snapshot_bytes(), before);
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
    assert_eq!(h.game.borrow().calls.len(), contacts);
    Ok(())
}

#[test]
fn monitor_policy_is_idempotent_and_cannot_be_redeployed_with_later_deadline() -> Result<()> {
    let (mut h, mut io) = completed(sparse_levels()?)?;
    let first = start(&mut h, &mut io)?;
    success(&first);
    let id = spec(&h)?.id();
    let bytes = io.history.snapshot_bytes();
    h.c.anchor.tick = GameTick(TICK + 100);
    let again = start(&mut h, &mut io)?;
    success(&again);
    assert_eq!(monitor_result(&again)["monitor_id"], id.to_string());
    assert_eq!(monitor_result(&again)["start_tick"], TICK);
    assert_eq!(io.history.snapshot_bytes(), bytes);
    let action = request(&h, TICK + 2000);
    let replacement = run(&mut h, &mut io, "fortress.plan", action)?;
    assert_eq!(replacement["result"]["error"]["code"], "conflict");
    assert_eq!(io.creations, 1);
    assert_eq!(io.reads, 0);
    assert_eq!(io.history.snapshot_bytes(), bytes);
    assert_eq!(spec(&h)?.policy().deadline_tick(), TICK + 1000);
    let blueprint = h.definition()?.id();
    for raw in [
        json!({"blueprint_id":blueprint.to_string(),"deadline_tick":TICK+1000,"unknown":1})
            .to_string(),
        json!({"blueprint_id":blueprint.to_string(),"deadline_tick":TICK+1000,"stable_ticks":null})
            .to_string(),
        format!(
            "{{\"blueprint_id\":\"{blueprint}\",\"deadline_tick\":{},\"deadline_tick\":{}}}",
            TICK + 1000,
            TICK + 2000
        ),
    ] {
        assert!(monitor::Request::parse(&raw).is_err());
    }
    Ok(())
}

#[test]
fn monitor_pages_cover_last_target_and_reject_previous_history_head() -> Result<()> {
    let layout = Layout::new(vec![BlueprintPart {
        origin: [2, 2, 2],
        size: [17, 1, 1],
    }])?;
    let (mut h, mut io) = completed(layout)?;
    success(&start(&mut h, &mut io)?);
    let id = spec(&h)?.id();
    let old = h.state.monitor.as_ref().ok_or_else(exhausted)?.head();
    let mut raw = capture(&h, TICK)?;
    replace(&mut raw, [18, 2, 2], Cell::Hidden)?;
    let value = sample(&mut h, &mut io, raw)?;
    success(&value);
    let first = monitor_result(&value);
    assert_eq!(first["status"], "unknown");
    assert_eq!(first["counts"]["matched"], 16);
    assert_eq!(first["counts"]["hidden"], 1);
    assert_eq!(
        first["target_page"]["tiles"]
            .as_array()
            .ok_or_else(exhausted)?
            .len(),
        16
    );
    assert_eq!(first["target_page"]["next_offset"], 16);
    let head = h.state.monitor.as_ref().ok_or_else(exhausted)?.head();
    let action = Ok(Action::Query(Query::BlueprintMonitor {
        monitor_id: id.to_string(),
        expected_head: Some(head.to_string()),
        offset: Some(16),
        limit: Some(16),
    }));
    let last = run(&mut h, &mut io, "fortress.query", action)?;
    success(&last);
    assert_eq!(
        monitor_result(&last)["target_page"]["tiles"],
        json!([
            {"coordinate":[18,2,2],"presence":"hidden","matches":false}
        ])
    );
    assert_eq!(
        monitor_result(&last)["target_page"]["next_offset"],
        Value::Null
    );
    let action = Ok(Action::Query(Query::BlueprintMonitor {
        monitor_id: id.to_string(),
        expected_head: Some(old.to_string()),
        offset: Some(16),
        limit: Some(16),
    }));
    let stale = run(&mut h, &mut io, "fortress.query", action)?;
    assert_eq!(stale["result"]["error"]["code"], "stale_anchor");
    assert_eq!(io.reads, 1);
    Ok(())
}

#[test]
fn monitor_routing_is_exclusive_and_source_recovery_inputs_are_closed() -> Result<()> {
    let id = Digest32::of_bytes(b"original monitor identity");
    let head = Digest32::of_bytes(b"acknowledged monitor history");
    let raw = json!({"blueprint_id":id.to_string(),"deadline_tick":TICK + 1000}).to_string();
    let planned = plan_with_monitor_action(None, None, None, None, Some(raw.clone()))?;
    let Action::StartMonitor(request) = planned else {
        return Err(exhausted());
    };
    assert_eq!(request.blueprint_id, id.to_string());
    assert_eq!(request.policy()?.deadline_tick(), TICK + 1000);
    assert_eq!(request.policy()?.stable_ticks(), 10);
    assert_eq!(request.policy()?.required_samples(), 2);
    let blueprint =
        String::from_utf8(sparse_levels()?.canonical_bytes().to_vec()).map_err(|_| exhausted())?;
    for (key, witness, hidden, parent) in [
        (Some("child".into()), None, None, None),
        (None, Some(id.to_string()), None, None),
        (None, None, Some(false), None),
        (None, None, None, Some(blueprint.clone())),
    ] {
        assert!(plan_with_monitor_action(key, witness, hidden, parent, Some(raw.clone())).is_err());
    }
    assert!(matches!(
        plan_with_monitor_action(None, None, None, Some(blueprint), None)?,
        Action::Blueprint(_)
    ));
    assert!(matches!(
        plan_with_monitor_action(
            Some("child".into()),
            Some(id.to_string()),
            Some(false),
            None,
            None
        )?,
        Action::Plan { .. }
    ));
    assert!(
        matches!(wait_action(None, None, Some(id.to_string()))?, Action::SampleMonitor(value) if value == id)
    );
    assert!(
        matches!(wait_action(Some("child".into()), Some(id.to_string()), None)?, Action::Wait(key, value) if key == "child" && value == id)
    );
    for (key, plan, monitor) in [
        (None, None, None),
        (Some("child".into()), None, None),
        (None, Some(id.to_string()), None),
        (Some("child".into()), None, Some(id.to_string())),
        (None, Some(id.to_string()), Some(id.to_string())),
        (
            Some("child".into()),
            Some(id.to_string()),
            Some(id.to_string()),
        ),
        (None, None, Some("replacement-goal".into())),
    ] {
        assert!(wait_action(key, plan, monitor).is_err());
    }
    let query = json!({"mode":"blueprint_monitor","monitor_id":id.to_string(),
        "expected_head":head.to_string(),"offset":16,"limit":16});
    assert!(
        matches!(Query::parse(&query.to_string())?, Query::BlueprintMonitor {
        monitor_id, expected_head: Some(pin), offset: Some(16), limit: Some(16),
    } if monitor_id == id.to_string() && pin == head.to_string())
    );
    for (name, value) in [
        ("offset", json!(512)),
        ("limit", json!(17)),
        ("limit", json!(0)),
        ("expected_head", Value::Null),
        ("monitor_id", json!("replacement-goal")),
        ("goal_id", json!(id.to_string())),
        ("original", json!(true)),
        ("mode", json!("monitor")),
    ] {
        let mut changed = query.clone();
        changed[name] = value;
        assert!(Query::parse(&changed.to_string()).is_err());
    }
    assert!(
        Query::parse(&format!(
            "{{\"mode\":\"blueprint_monitor\",\"monitor_id\":\"{id}\",\"monitor_id\":\"{head}\"}}"
        ))
        .is_err()
    );

    let configured = json!({"path":"/private/dig/monitor","initialize":true,
        "source":{"endpoint":"127.0.0.1:5000","generation":29,
            "df_version":"test-df","dfhack_version":"test-dfhack"}});
    let parsed = monitor::Config::parse(&configured.to_string())?;
    assert!(parsed.initialize);
    assert!(parsed.recovery.is_none());
    assert_eq!(parsed.source.generation(), 29);
    let mut reopening = configured.clone();
    reopening["initialize"] = json!(false);
    assert!(monitor::Config::parse(&reopening.to_string()).is_err());
    reopening["recovery"] = json!({"monitor_id":id.to_string(),"head":head.to_string()});
    let retained = monitor::Config::parse(&reopening.to_string())?;
    assert!(!retained.initialize);
    let recovery = retained.recovery.ok_or_else(exhausted)?;
    assert_eq!(recovery.monitor_id, id);
    assert_eq!(recovery.head, head);
    let mut conflicting = reopening.clone();
    conflicting["initialize"] = json!(true);
    assert!(monitor::Config::parse(&conflicting.to_string()).is_err());
    let mut zero = reopening;
    zero["recovery"]["head"] = json!(Digest32::ZERO.to_string());
    assert!(monitor::Config::parse(&zero.to_string()).is_err());
    for (name, value) in [
        ("generation", json!(0)),
        ("generation", json!(u64::MAX)),
        ("endpoint", json!("192.0.2.1:5000")),
        ("endpoint", json!("localhost:5000")),
        ("endpoint", json!("127.0.0.1:0")),
        ("df_version", json!("")),
        ("protocol", json!("1.8")),
    ] {
        let mut changed = configured.clone();
        changed["source"][name] = value;
        assert!(monitor::Config::parse(&changed.to_string()).is_err());
    }
    for path in [
        "relative/monitor",
        "/private/../monitor",
        "/private//monitor",
        "/private/./monitor",
    ] {
        let mut changed = configured.clone();
        changed["path"] = json!(path);
        assert!(monitor::Config::parse(&changed.to_string()).is_err());
    }
    Ok(())
}
