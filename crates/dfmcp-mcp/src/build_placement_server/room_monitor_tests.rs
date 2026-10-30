//! Joint room MCP lifecycle checks use real private parent/native/monitor
//! histories and independent protocol fixtures. The only injected operation
//! is the existing query-only source bracket; no live fortress is implied.
use super::*;
use dfmcp_adapter::furniture_allocation::{Kind, Request, Slot};
use dfmcp_adapter::live_map::LiveMapObservation;
use dfmcp_adapter::live_operations::LiveOperationsState;
use dfmcp_adapter::room_condition::{self, Definition, Sample};
use dfmcp_adapter::room_intent::{RoomIntent, TerrainRequirements};
use dfmcp_world::map_region::{Cell, MapRegion, Region, Shape, Tile};

#[derive(Clone, Copy, Default)]
enum TerrainFault {
    #[default]
    None,
    WallMissing,
    FloorHidden,
    FloorWet,
}

fn room_sample(
    definition: &Definition,
    tick: u64,
    complete: bool,
    fault: TerrainFault,
) -> Result<Sample> {
    let binding = definition.origin().definition().binding();
    let terrain = definition.room().terrain();
    let region = terrain.region();
    let floor = Tile {
        native_tiletype: 219,
        shape: Shape::Floor,
        liquid_depth: 0,
        magma: false,
        traffic: 0,
        dig_designation: 0,
        building_occupancy: 0,
        unit_occupancy: 0,
        walkable_region: 1,
        temperature_1: 10015,
        temperature_2: 10015,
    };
    let mut map = LiveMapObservation {
        bridge_generation: 87,
        df_version: "test-df".into(),
        dfhack_version: "test-dfhack".into(),
        year: (tick / 403_200) as u32,
        year_tick: (tick % 403_200) as u32,
        paused: true,
        site_id: binding.fortress().site(),
        world_folder: binding.fortress().folder().into(),
        map_dimensions: binding.dimensions(),
        map: MapRegion {
            region,
            cells: vec![Cell::Visible(floor); region.volume().map_err(|_| exhausted())?],
        },
    };
    for &position in terrain.walls() {
        let mut wall = floor;
        wall.shape = Shape::Wall;
        map.map.cells[region.index(position).ok_or_else(exhausted)?] = Cell::Visible(wall);
    }
    for record in definition.goal().records() {
        let mut occupied = floor;
        occupied.building_occupancy = 1;
        map.map.cells[region
            .index(record.insertion().ok_or_else(exhausted)?.position())
            .ok_or_else(exhausted)?] = Cell::Visible(occupied);
    }
    match fault {
        TerrainFault::None => {}
        TerrainFault::WallMissing => {
            let position = *terrain.walls().first().ok_or_else(exhausted)?;
            map.map.cells[region.index(position).ok_or_else(exhausted)?] = Cell::Visible(floor);
        }
        TerrainFault::FloorHidden => {
            let position = *terrain.floors().last().ok_or_else(exhausted)?;
            map.map.cells[region.index(position).ok_or_else(exhausted)?] = Cell::Hidden;
        }
        TerrainFault::FloorWet => {
            let position = *terrain.floors().last().ok_or_else(exhausted)?;
            let mut wet = floor;
            wet.liquid_depth = 1;
            map.map.cells[region.index(position).ok_or_else(exhausted)?] = Cell::Visible(wet);
        }
    }
    let manifest = Manifest {
        generation: 87,
        df_version: "test-df".into(),
        dfhack_version: "test-dfhack".into(),
    };
    let raw = map.encode_payload()?;
    Ok(Sample {
        before: manifest.clone(),
        before_capture: raw.clone(),
        furnishings: linked_sample(definition.original(), tick, complete)?,
        after: manifest,
        after_capture: raw,
    })
}

enum CheckpointFault {
    Reject,
    Corrupt(PathBuf),
    Remove(PathBuf),
}

struct JointIo {
    tick: u64,
    complete: bool,
    terrain: TerrainFault,
    samples: usize,
    checkpoints: usize,
    floor: u64,
    fail_next_sample: bool,
    sampled_this_turn: bool,
    checkpoint_fault: Option<CheckpointFault>,
    fault_on_checkpoint: Option<usize>,
    queued: VecDeque<Sample>,
}
impl Default for JointIo {
    fn default() -> Self {
        Self {
            tick: TICK + 10,
            complete: true,
            terrain: TerrainFault::None,
            samples: 0,
            checkpoints: 0,
            floor: 0,
            fail_next_sample: false,
            sampled_this_turn: false,
            checkpoint_fault: None,
            fault_on_checkpoint: None,
            queued: VecDeque::new(),
        }
    }
}
impl room_monitor::Io for JointIo {
    fn sample(
        &mut self,
        binding: &BuildBinding,
        definition: &Definition,
        _: &OperationContext,
    ) -> Result<Sample> {
        assert_eq!(binding, definition.origin().definition().binding());
        self.samples += 1;
        self.sampled_this_turn = true;
        self.floor = self.floor.max(self.tick);
        if std::mem::take(&mut self.fail_next_sample) {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "injected room bracket lost after a consumed source clock",
            ));
        }
        if let Some(sample) = self.queued.pop_front() {
            return Ok(sample);
        }
        room_sample(definition, self.tick, self.complete, self.terrain)
    }
    fn observed_tick_floor(&self) -> u64 {
        self.floor
    }
    fn checkpoint(&mut self, _: &OperationContext) -> Result<()> {
        self.checkpoints += 1;
        if self.sampled_this_turn || self.fault_on_checkpoint == Some(self.checkpoints) {
            match self.checkpoint_fault.take() {
                Some(CheckpointFault::Reject) => {
                    return Err(error(
                        ErrorCode::CapabilityDenied,
                        "injected room publication boundary revoked",
                    ));
                }
                Some(CheckpointFault::Corrupt(path)) => {
                    let mut raw = fs::read(&path).map_err(storage_error)?;
                    let byte = raw.last_mut().ok_or_else(exhausted)?;
                    *byte ^= 1;
                    fs::write(path, raw).map_err(storage_error)?;
                }
                Some(CheckpointFault::Remove(path)) => {
                    fs::remove_file(path).map_err(storage_error)?;
                }
                None => {}
            }
        }
        Ok(())
    }
}

struct RoomHarness {
    state: ClockState,
    config: Config,
    io: JointIo,
    directory: Directory,
}

fn room_timing() -> Timing {
    Timing {
        deadline: TICK + 1000,
        interval: 10,
        stable_samples: 2,
        stable_span: 10,
        max_gap: 1200,
        max_observations: 32,
    }
}

fn setup_room(count: usize) -> Result<RoomHarness> {
    setup_room_with_padding(count, false)
}

fn setup_room_with_padding(count: usize, full_map: bool) -> Result<RoomHarness> {
    let directory = Directory::new()?;
    let originals = original_goal(count)?;
    let first = originals.records()[0].plan().before();
    let binding = BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        "test-df",
        "test-dfhack",
        first,
    )?;
    let positions = originals
        .records()
        .iter()
        .map(|record| record.plan().before().selection().target())
        .collect::<Vec<_>>();
    let x = positions.iter().map(|p| p[0]).min().ok_or_else(exhausted)?;
    let y = positions.iter().map(|p| p[1]).min().ok_or_else(exhausted)?;
    let xmax = positions.iter().map(|p| p[0]).max().ok_or_else(exhausted)?;
    let ymax = positions.iter().map(|p| p[1]).max().ok_or_else(exhausted)?;
    let z = positions[0][2];
    let region = if full_map {
        Region {
            origin: [0, 0, z],
            size: [64, 64, 1],
        }
    } else {
        Region {
            origin: [x - 1, y - 1, z],
            size: [xmax - x + 3, ymax - y + 3, 1],
        }
    };
    let mut floors = positions;
    floors.push([x, ymax + 1, z]);
    let request = FurnitureRequest::new(
        first.folder().into(),
        first.site(),
        Request {
            slots: originals
                .records()
                .iter()
                .enumerate()
                .map(|(i, record)| {
                    let selection = record.plan().before().selection();
                    Slot {
                        name: format!("s{i:02}"),
                        kind: match selection.kind() {
                            BuildKind::Bed => Kind::Bed,
                            BuildKind::Chair => Kind::Chair,
                            BuildKind::Table => Kind::Table,
                        },
                        target: selection.target(),
                        after: Vec::new(),
                        material: None,
                        subtype: None,
                        max_distance: dfmcp_adapter::furniture_allocation::MAX_DISTANCE,
                    }
                })
                .collect(),
            excluded_items: Vec::new(),
        },
    )?;
    let room = RoomIntent::new(
        request.clone(),
        TerrainRequirements::new(region, floors, vec![[x - 1, y - 1, z]])?,
    )?;
    let corpus = LinkedSample::decode(&hex(include_str!(
        "../../../dfmcp-adapter/src/construction_plan/testdata/python_sample_32.hex"
    ))?)?;
    let mut supply = LiveOperationsObservation::decode_profile(
        &corpus.capture,
        corpus.operations.generation,
        corpus.operations.df_version,
        corpus.operations.dfhack_version,
        OperationsProfile::PagedV1_4,
    )?;
    supply.jobs.year = (first.tick() / 403_200) as u32;
    supply.jobs.year_tick = (first.tick() % 403_200) as u32;
    supply.jobs.next_job_id = first.next_job_id();
    supply.jobs.jobs.clear();
    supply.next_building_id = first.next_building_id();
    supply.buildings.clear();
    supply.attachments.clear();
    supply.items.retain(|item| {
        originals
            .records()
            .iter()
            .any(|record| record.plan().before().selection().item_id() == item.native_id)
    });
    for item in &mut supply.items {
        let record = originals
            .records()
            .iter()
            .find(|record| record.plan().before().selection().item_id() == item.native_id)
            .ok_or_else(exhausted)?;
        let p = record.plan().before().selection().target();
        item.raw_position = MapCoord::new(p[0] as i32, p[1] as i32, p[2] as i32);
        item.flags = 64;
        item.holder_building_native_id = None;
        item.container_native_id = None;
    }
    let mut available = LiveOperationsState::with_profile(OperationsProfile::PagedV1_4);
    available.publish(supply)?;
    let mut c = context(
        SessionId::new(1700 + count as u128),
        RequestId::new(1),
        first.fortress_id(),
        first.tick(),
        budget(),
        BuildMode::Control,
        true,
    );
    c.anchor = available.snapshot().ok_or_else(exhausted)?.anchor();
    c.budget.max_entities = 100_000;
    let handoff = Handoff::allocate(
        &available,
        &c,
        binding.endpoint(),
        &request,
        dfmcp_adapter::furniture_supply::MAX_WORK,
    )?
    .handoff
    .ok_or_else(exhausted)?;
    let mut raw = b"DFMBJ019".to_vec();
    field32(&mut raw, &binding.canonical_bytes());
    raw.extend_from_slice(&[93; 32]);
    let journal_id = domain_hash(b"dfmcp-build-journal/1", &raw);
    raw.extend_from_slice(journal_id.as_bytes());
    let batch = BatchDefinition::from_room_handoff(room, handoff, binding.clone(), journal_id)?;
    let mut head = journal_id;
    let mut sequence = 0;
    for (step, record) in batch.plan().ordered_steps().zip(originals.records()) {
        assert_eq!(step.selection, record.plan().before().selection());
        let key = batch.key(step);
        let prepared = rekey(record, &key, false)?;
        let placed = rekey(record, &key, true)?;
        for (stage, native) in [
            (0, None),
            (1, Some(&prepared)),
            (2, Some(&prepared)),
            (5, Some(&placed)),
        ] {
            append_placement(
                &mut raw,
                &mut head,
                &mut sequence,
                placed.plan(),
                stage,
                native,
            );
        }
    }
    let mut config = config()?;
    config.path = directory.file("placements");
    config.batch_path = Some(directory.file("batch"));
    config.completion_path = Some(directory.file("completion"));
    config.room_completion_path = Some(directory.file("room-completion"));
    config.fortress = first.fortress().clone();
    config.clock = None;
    c.anchor.tick = GameTick(TICK + 1);
    write_private(&config.path, &raw)?;
    let native =
        open_private_build_exact(&config.path, &c, BuildMode::Control, Some(binding), false)?;
    let mut state = State::new(BuildSession::new(native, &c)?, &c, &config)?;
    state.batch = Some(open_private_batch(
        config.batch_path.as_ref().ok_or_else(exhausted)?,
        &c,
        BuildMode::Control,
        Some(batch),
    )?);
    Ok(RoomHarness {
        state,
        config,
        io: JointIo::default(),
        directory,
    })
}

impl RoomHarness {
    fn call(&mut self, action: room_monitor::Action) -> Result<Value> {
        let op = match action {
            room_monitor::Action::Start(_) | room_monitor::Action::Inspect => "fortress.query",
            room_monitor::Action::Sample => "fortress.wait",
            room_monitor::Action::Cancel => "fortress.cancel",
        };
        let c = self.state.context(false, None)?;
        self.call_with(c, op, action)
    }
    fn call_with(
        &mut self,
        c: OperationContext,
        op: &str,
        action: room_monitor::Action,
    ) -> Result<Value> {
        self.io.sampled_this_turn = false;
        let output = room_monitor::run(
            &mut self.state,
            &self.config,
            c,
            op,
            action,
            Instant::now(),
            &mut self.io,
        );
        assert!(output.len() as u64 <= OUTPUT_BYTES);
        serde_json::from_str(&output).map_err(|_| exhausted())
    }
    fn start(&mut self) -> Result<Value> {
        self.call(room_monitor::Action::Start(room_timing()))
    }
    fn monitor(&self) -> Result<&room_monitor::Monitor> {
        self.state
            .room_completion
            .as_ref()
            .ok_or_else(missing_fixture)
    }
    fn definition(&self) -> Result<&Definition> {
        Ok(self.monitor()?.store.definition())
    }
    fn progress(&self) -> Result<&room_condition::Progress> {
        Ok(self.monitor()?.store.progress())
    }
    fn reopen(&mut self) -> Result<()> {
        let pin = self.monitor()?.store.recovery();
        self.state.room_completion = None;
        self.state.room_recovery = Some(pin);
        let c = self.state.context(false, None)?;
        let view = self.state.control.inventory(&c)?;
        room_monitor::reopen(
            &mut self.state,
            &self.config,
            &c,
            &mut Work::new(&c, Instant::now())?,
            &view,
        )
    }
    fn into_offline(self) -> Result<Self> {
        let recovery = self.monitor()?.store.recovery();
        let binding = self.state.binding.clone();
        let tick = self
            .state
            .authority_tick
            .max(self.state.control.high_tick());
        let id = SessionId::new(
            self.state
                .id
                .get()
                .checked_add(10_000)
                .ok_or_else(exhausted)?,
        );
        let work_budget = self.state.budget;
        let Self {
            state,
            mut config,
            io,
            directory,
        } = self;
        // Release every old lock; the new owner follows the public Offline
        // profile instead of merely changing a monitor's configuration label.
        drop(state);
        config.mode = BuildMode::Offline;
        config.room_completion_recovery = Some(recovery);
        let c = context(
            id,
            RequestId::new(1),
            binding.fortress().fortress_id(),
            tick,
            work_budget,
            BuildMode::Offline,
            false,
        );
        let native =
            open_private_build_exact(&config.path, &c, BuildMode::Offline, Some(binding), false)?;
        let mut state: ClockState = State::new(BuildSession::new(native, &c)?, &c, &config)?;
        state.batch = Some(open_private_batch(
            config.batch_path.as_ref().ok_or_else(exhausted)?,
            &c,
            BuildMode::Offline,
            None,
        )?);
        let view = state.control.inventory(&c)?;
        room_monitor::reopen(
            &mut state,
            &config,
            &c,
            &mut Work::new(&c, Instant::now())?,
            &view,
        )?;
        Ok(Self {
            state,
            config,
            io,
            directory,
        })
    }
}

fn room_result(result: &Value) -> &Value {
    &result["result"]["room_completion"]
}

#[test]
fn joint_room_mcp_retains_every_original_target_and_publishes_one_shared_condition() -> Result<()> {
    for count in [1, 32] {
        let mut h = setup_room(count)?;
        let original_native = fs::read(&h.config.path).map_err(storage_error)?;
        let original_parent =
            fs::read(h.config.batch_path.as_ref().ok_or_else(exhausted)?).map_err(storage_error)?;
        let started = h.start()?;
        assert_eq!(started["result"]["ok"], true, "{started}");
        assert_eq!(room_result(&started)["selection_count"], count);
        assert_eq!(room_result(&started)["observations"], 0);
        assert_eq!(room_result(&started)["sampled_condition_satisfied"], false);
        assert_eq!(
            h.io.samples, 0,
            "starting a monitor cannot acquire a sample"
        );
        assert!(h.state.completion.is_none());
        let expected = h.definition()?.clone();
        assert_eq!(expected.goal().records().len(), count);
        assert_eq!(expected.room().terrain().floors().len(), count + 1);

        let candidate = h.call(room_monitor::Action::Sample)?;
        assert_eq!(candidate["result"]["ok"], true, "{candidate}");
        assert_eq!(h.progress()?.streak, 1);
        assert_eq!(h.progress()?.phase, "candidate");
        assert!(!h.progress()?.terminal());
        assert_eq!(room_result(&candidate)["fresh_joint_sample_acquired"], true);
        assert_eq!(
            room_result(&candidate)["sampled_condition_satisfied"],
            false
        );
        h.io.tick += 10;
        let completed = h.call(room_monitor::Action::Sample)?;
        assert_eq!(completed["result"]["ok"], true, "{completed}");
        let result = room_result(&completed);
        assert_eq!(result["phase"], "satisfied");
        assert_eq!(result["sampled_condition_satisfied"], true);
        assert_eq!(result["fresh_joint_sample_acquired"], true);
        assert_eq!(result["assessments_complete"], true);
        assert_eq!(result["condition_met_count"], count);
        assert_eq!(result["terrain"]["floors"]["matched"], count + 1);
        assert_eq!(result["terrain"]["walls"]["matched"], 1);
        assert_eq!(result["terrain"]["furnishings"]["matched"], count);
        assert_eq!(
            result["room_intent_digest"],
            expected.room().digest().to_string()
        );
        assert_eq!(result["source"]["map_generation"], 87);
        assert_eq!(result["atomic_cross_profile_snapshot_proven"], false);
        assert_eq!(result["current_room_usability_proven"], false);
        let rows = result["assessments"].as_array().ok_or_else(exhausted)?;
        assert_eq!(rows.len(), count);
        for receipt in expected.goal().records() {
            assert!(rows.iter().any(|row| {
                row["key"] == receipt.plan().key()
                    && row["receipt_digest"] == receipt.receipt().to_string()
                    && row["building_id"]
                        == json!(receipt.insertion().map(|insertion| insertion.building_id()))
            }));
        }
        let historical = h.call(room_monitor::Action::Inspect)?;
        assert_eq!(historical["result"]["ok"], true, "{historical}");
        assert_eq!(
            room_result(&historical)["fresh_joint_sample_acquired"],
            false
        );
        assert_eq!(h.io.samples, 2);
        assert_eq!(
            fs::read(&h.config.path).map_err(storage_error)?,
            original_native
        );
        assert_eq!(
            fs::read(h.config.batch_path.as_ref().ok_or_else(exhausted)?).map_err(storage_error)?,
            original_parent
        );
    }
    Ok(())
}

#[test]
fn room_monitor_never_combines_furniture_and_terrain_from_different_acquisitions() -> Result<()> {
    let mut h = setup_room(32)?;
    assert_eq!(h.start()?["result"]["ok"], true);
    h.io.complete = false;
    let terrain_only = h.call(room_monitor::Action::Sample)?;
    assert_eq!(terrain_only["result"]["ok"], true, "{terrain_only}");
    assert!(h.progress()?.terrain.condition_met());
    assert_eq!(h.progress()?.condition_met_count(), 0);
    assert_eq!(h.progress()?.streak, 0);
    assert_eq!(
        room_result(&terrain_only)["sampled_condition_satisfied"],
        false
    );
    h.io.tick += 10;
    h.io.complete = true;
    h.io.terrain = TerrainFault::WallMissing;
    let furniture_only = h.call(room_monitor::Action::Sample)?;
    assert_eq!(furniture_only["result"]["ok"], true, "{furniture_only}");
    assert_eq!(h.progress()?.condition_met_count(), 32);
    assert_eq!(h.progress()?.terrain.walls.wrong_shape, 1);
    assert_eq!(h.progress()?.streak, 0);
    h.io.terrain = TerrainFault::None;
    h.io.tick += 10;
    assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
    assert_eq!(h.progress()?.streak, 1);
    h.io.tick += 10;
    let joint = h.call(room_monitor::Action::Sample)?;
    assert_eq!(joint["result"]["ok"], true, "{joint}");
    assert_eq!(room_result(&joint)["sampled_condition_satisfied"], true);
    assert_eq!(h.progress()?.observations, 4);
    Ok(())
}

#[test]
fn hidden_wet_and_changed_same_tick_room_cells_withdraw_the_whole_streak() -> Result<()> {
    for fault in [
        TerrainFault::FloorHidden,
        TerrainFault::FloorWet,
        TerrainFault::WallMissing,
    ] {
        let mut h = setup_room(2)?;
        assert_eq!(h.start()?["result"]["ok"], true);
        assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
        assert_eq!(h.progress()?.streak, 1);
        h.io.terrain = fault;
        let changed = h.call(room_monitor::Action::Sample)?;
        assert_eq!(changed["result"]["ok"], true, "{changed}");
        assert_eq!(h.progress()?.streak, 0);
        assert_eq!(room_result(&changed)["sampled_condition_satisfied"], false);
        h.io.terrain = TerrainFault::None;
        let restored_same_tick = h.call(room_monitor::Action::Sample)?;
        assert_eq!(
            restored_same_tick["result"]["ok"], true,
            "{restored_same_tick}"
        );
        assert_eq!(h.progress()?.streak, 0);
        h.io.tick += 10;
        assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
        assert_eq!(h.progress()?.streak, 1);
        h.io.tick += 10;
        assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
        assert_eq!(h.progress()?.phase, "satisfied");
    }
    Ok(())
}

#[test]
fn joint_room_restart_preserves_full_origin_and_resets_unfinished_continuity() -> Result<()> {
    let mut h = setup_room(32)?;
    assert_eq!(h.start()?["result"]["ok"], true);
    assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
    let definition = h.definition()?.canonical_bytes().to_vec();
    let pin = h.monitor()?.store.recovery();
    let source = h.monitor()?.store.first_source().cloned();
    assert_eq!(h.progress()?.streak, 1);
    h.reopen()?;
    assert_eq!(h.definition()?.canonical_bytes(), definition);
    assert_eq!(h.monitor()?.store.recovery(), pin);
    assert_eq!(h.monitor()?.store.first_source(), source.as_ref());
    assert_eq!(h.progress()?.streak, 0);
    assert_eq!(h.definition()?.goal().records().len(), 32);
    let historical = h.call(room_monitor::Action::Inspect)?;
    assert_eq!(historical["result"]["ok"], true, "{historical}");
    assert_eq!(
        room_result(&historical)["fresh_joint_sample_acquired"],
        false
    );
    h.io.tick += 10;
    assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
    assert_eq!(h.progress()?.streak, 1);
    h.io.tick += 10;
    assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
    assert_eq!(h.progress()?.phase, "satisfied");
    assert_eq!(h.io.samples, 3);
    Ok(())
}

#[test]
fn joint_room_reopen_rejects_rollback_below_an_acknowledged_history_head() -> Result<()> {
    let mut h = setup_room(2)?;
    assert_eq!(h.start()?["result"]["ok"], true);
    assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
    let path = h
        .config
        .room_completion_path
        .clone()
        .ok_or_else(exhausted)?;
    let earlier = fs::read(&path).map_err(storage_error)?;
    h.io.tick += 10;
    h.io.terrain = TerrainFault::WallMissing;
    assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
    let acknowledged = h.monitor()?.store.recovery();
    h.state.room_completion = None;
    h.state.room_recovery = Some(acknowledged);
    fs::write(&path, earlier).map_err(storage_error)?;
    let c = h.state.context(false, None)?;
    let view = h.state.control.inventory(&c)?;
    let outcome = room_monitor::reopen(
        &mut h.state,
        &h.config,
        &c,
        &mut Work::new(&c, Instant::now())?,
        &view,
    );
    assert!(
        outcome.is_err(),
        "an earlier valid prefix cannot erase acknowledged evidence"
    );
    assert_eq!(h.state.room_recovery, Some(acknowledged));
    assert_eq!(h.io.samples, 2);
    Ok(())
}

#[test]
fn failed_joint_source_retains_its_clock_before_local_cancel_and_restart() -> Result<()> {
    let mut h = setup_room(2)?;
    assert_eq!(h.start()?["result"]["ok"], true);
    assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
    assert_eq!(h.progress()?.streak, 1);
    h.io.tick = TICK + 80;
    h.io.fail_next_sample = true;
    let failure = h.call(room_monitor::Action::Sample)?;
    assert_eq!(failure["result"]["ok"], false, "{failure}");
    assert!(h.state.authority_tick >= TICK + 80);
    assert!(h.monitor()?.store.authority_tick_floor() >= TICK + 80);
    assert_eq!(h.progress()?.streak, 0);
    assert_eq!(room_result(&failure)["sampled_condition_satisfied"], false);
    assert!(
        room_result(&failure)["authority_tick_floor"]
            .as_u64()
            .is_some_and(|v| v >= TICK + 80)
    );
    let cancelled = h.call(room_monitor::Action::Cancel)?;
    assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
    assert_eq!(h.progress()?.phase, "cancelled");
    h.reopen()?;
    assert_eq!(h.progress()?.phase, "cancelled");
    assert!(h.monitor()?.store.authority_tick_floor() >= TICK + 80);
    assert_eq!(h.io.samples, 2);
    Ok(())
}

#[test]
fn room_source_clock_expiry_cannot_publish_with_stale_caller_query_authority() -> Result<()> {
    let mut h = setup_room(2)?;
    assert_eq!(h.start()?["result"]["ok"], true);
    let mut c = h.state.context(false, None)?;
    for grant in &mut c.grants {
        if grant.capability == Capability::Query {
            grant.expires_at_tick = Some(GameTick(TICK + 50));
        }
    }
    h.io.tick = TICK + 80;
    let denied = h.call_with(c, "fortress.wait", room_monitor::Action::Sample)?;
    assert_eq!(denied["result"]["ok"], false, "{denied}");
    assert_eq!(h.io.samples, 1);
    assert!(h.state.authority_tick >= TICK + 80);
    assert!(h.monitor()?.store.authority_tick_floor() >= TICK + 80);
    assert_eq!(h.progress()?.observations, 0);
    assert!(denied["result"].get("room_completion").is_none());
    let disclosed = h.call(room_monitor::Action::Inspect)?;
    assert_eq!(disclosed["result"]["ok"], true, "{disclosed}");
    assert_eq!(
        room_result(&disclosed)["sampled_condition_satisfied"],
        false
    );
    assert!(
        room_result(&disclosed)["authority_tick_floor"]
            .as_u64()
            .is_some_and(|v| v >= TICK + 80)
    );
    Ok(())
}

#[test]
fn late_room_publication_failure_withdraws_freshness_and_preserves_recovery_identity() -> Result<()>
{
    for fault in 0..4 {
        let mut h = setup_room(2)?;
        assert_eq!(h.start()?["result"]["ok"], true);
        let id = h.definition()?.id();
        h.io.checkpoint_fault = Some(match fault {
            0 => CheckpointFault::Reject,
            1 => CheckpointFault::Corrupt(
                h.config
                    .room_completion_path
                    .clone()
                    .ok_or_else(exhausted)?,
            ),
            2 => CheckpointFault::Remove(h.config.path.clone()),
            _ => CheckpointFault::Remove(h.config.batch_path.clone().ok_or_else(exhausted)?),
        });
        let failure = h.call(room_monitor::Action::Sample)?;
        assert_eq!(failure["result"]["ok"], false, "{failure}");
        assert_eq!(h.io.samples, 1);
        assert_eq!(h.state.room_recovery.map(|pin| pin.monitor_id), Some(id));
        assert_eq!(room_result(&failure)["sampled_condition_satisfied"], false);
        assert_ne!(room_result(&failure)["fresh_joint_sample_acquired"], true);
        assert_eq!(h.progress()?.streak, 0);
        assert!(h.state.authority_tick >= h.io.tick);
    }
    Ok(())
}

#[test]
fn query_only_room_cancel_survives_loss_of_both_original_files() -> Result<()> {
    let mut h = setup_room(32)?;
    assert_eq!(h.start()?["result"]["ok"], true);
    let id = h.definition()?.id();
    fs::remove_file(&h.config.path).map_err(storage_error)?;
    fs::remove_file(h.config.batch_path.as_ref().ok_or_else(exhausted)?).map_err(storage_error)?;
    h.state.grants.retain(|g| g.capability == Capability::Query);
    let cancelled = h.call(room_monitor::Action::Cancel)?;
    assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
    assert_eq!(h.progress()?.phase, "cancelled");
    assert_eq!(room_result(&cancelled)["origin_verified"], false);
    assert_eq!(room_result(&cancelled)["monitor_terminal"], true);
    assert_eq!(room_result(&cancelled)["monitor_id"], id.to_string());
    assert_eq!(room_result(&cancelled)["selection_count"], 32);
    assert_eq!(cancelled["result"]["game_mutation_dispatched"], false);
    assert_eq!(h.io.samples, 0);
    Ok(())
}

#[test]
fn room_native_acquisition_is_precluded_by_insufficient_budget_or_configured_clock_custody()
-> Result<()> {
    for blocked_clock in [false, true] {
        let mut h = setup_room(2)?;
        assert_eq!(h.start()?["result"]["ok"], true);
        let path = h
            .config
            .room_completion_path
            .clone()
            .ok_or_else(exhausted)?;
        let before = fs::read(&path).map_err(storage_error)?;
        if blocked_clock {
            let config = clock::Config {
                path: h.directory.file("unopened-clock"),
                initialize: false,
                binding: RunBinding::new(
                    h.state.binding.endpoint(),
                    RunManifest {
                        generation: 41,
                        df_version: "test-df".into(),
                        dfhack_version: "test-dfhack".into(),
                    },
                )?,
                recovery: None,
            };
            h.state.clock_config = Some(config.clone());
            h.config.clock = Some(config);
        } else {
            h.state.budget.max_bytes = OUTPUT_BYTES + 2 * VIEW_BYTES + 128 * 1024;
        }
        let refused = h.call(room_monitor::Action::Sample)?;
        assert_eq!(refused["result"]["ok"], false, "{refused}");
        assert_eq!(h.io.samples, 0);
        assert_eq!(fs::read(&path).map_err(storage_error)?, before);
        assert_ne!(room_result(&refused)["sampled_condition_satisfied"], true);
    }
    Ok(())
}

fn prepare_original_furniture_clock(h: &mut RoomHarness) -> Result<ClockIo> {
    let definition = h.definition()?.original().clone();
    h.state.budget.max_game_ticks = 1200;
    h.state.grants.push(CapabilityGrant {
        capability: Capability::ControlClock,
        scope: CapabilityScope {
            fortress_id: Some(h.state.binding.fortress().fortress_id()),
            ..CapabilityScope::default()
        },
        max_risk: RiskTier::Guarded,
        expires_at_tick: None,
        remaining_uses: None,
    });
    let c = h.state.context(true, None)?;
    let initial = linked_sample(&definition, TICK + 1, false)?;
    let mut store = open_private_monitor(
        h.config.completion_path.as_ref().ok_or_else(exhausted)?,
        &c,
        BuildMode::Control,
        Some(definition.clone()),
    )?;
    let mut admit = |_: &MonitorDefinition, _: &Progress| Ok(());
    let mut guard = |_: &Origin, _: &OperationContext| Ok(());
    store.begin_read(&c, &mut admit, &mut guard)?;
    store.publish_sample(&c, &initial, &mut admit, &mut guard)?;
    drop(store);
    let view = h.state.control.inventory(&c)?;
    completion::reopen(
        &mut h.state,
        &h.config,
        &c,
        &mut Work::new(&c, Instant::now())?,
        &view,
    )?;
    let manifest = RunManifest {
        generation: 41,
        df_version: "test-df".into(),
        dfhack_version: "test-dfhack".into(),
    };
    let config = clock::Config {
        path: h.directory.file("clock"),
        initialize: true,
        binding: RunBinding::new(h.state.binding.endpoint(), manifest.clone())?,
        recovery: None,
    };
    let mut io = ClockIo {
        game: Rc::new(RefCell::new(RunGame::new())),
        manifest,
        samples: VecDeque::new(),
        complete: false,
        acquisitions: 0,
        connections: 0,
        checkpoints: 0,
        deny_control: false,
        lose_parent_after_sample: None,
        opening: None,
    };
    h.state.clock_grants = clock_grants();
    let n = clock::native_context(&c, &h.state.clock_grants)?;
    let owner = clock::ClockIo::open(&mut io, &config, &definition, &initial.operations, &c, &n)?;
    h.state.clock_recovery = Some(owner.recovery());
    h.state.clock = Some(owner);
    h.state.clock_config = Some(config.clone());
    h.config.clock = Some(config);
    let c = h.state.context(true, None)?;
    let output = clock::run_action(
        &mut h.state,
        c,
        "fortress.plan",
        clock::Action::Prepare(clock::Request {
            monitor_id: definition.id(),
            game_ticks: 10,
            wall_millis: 5000,
        }),
        Instant::now(),
        &mut io,
    );
    let result: Value = serde_json::from_str(&output).map_err(|_| exhausted())?;
    assert_eq!(result["result"]["ok"], true, "{result}");
    assert!(
        h.state
            .clock
            .as_ref()
            .ok_or_else(exhausted)?
            .cached_view()
            .pending
    );
    Ok(io)
}

#[test]
fn retained_prepared_furniture_clock_blocks_joint_room_sampling_without_promoting_its_goal()
-> Result<()> {
    let mut h = setup_room(2)?;
    assert_eq!(h.start()?["result"]["ok"], true);
    let room_id = h.definition()?.id();
    let mut clock_io = prepare_original_furniture_clock(&mut h)?;
    let clock_id = h
        .state
        .completion
        .as_ref()
        .ok_or_else(exhausted)?
        .store
        .definition()
        .id();
    assert_ne!(room_id, clock_id);
    let before_clock = h.state.clock.as_ref().ok_or_else(exhausted)?.recovery();
    let before_room = h.monitor()?.store.recovery();
    let refused = h.call(room_monitor::Action::Sample)?;
    assert_eq!(refused["result"]["ok"], false, "{refused}");
    assert_eq!(h.io.samples, 0);
    assert_eq!(h.monitor()?.store.recovery(), before_room);
    assert_eq!(
        h.state.clock.as_ref().ok_or_else(exhausted)?.recovery(),
        before_clock
    );
    assert!(!clock_io.game.borrow().calls.contains(&"commit"));
    let c = h.state.context(true, None)?;
    let output = clock::run_action(
        &mut h.state,
        c,
        "fortress.cancel",
        clock::Action::Cancel(clock_id),
        Instant::now(),
        &mut clock_io,
    );
    let cancelled: Value = serde_json::from_str(&output).map_err(|_| exhausted())?;
    assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
    assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
    assert_eq!(h.io.samples, 1);
    assert_eq!(h.progress()?.streak, 1);
    assert_eq!(h.definition()?.id(), room_id);
    Ok(())
}

#[test]
fn lost_outer_response_breaks_unfinished_joint_room_continuity_before_the_next_sample() -> Result<()>
{
    for acknowledged in [false, true] {
        let mut h = setup_room(2)?;
        assert_eq!(h.start()?["result"]["ok"], true);
        assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
        assert_eq!(h.progress()?.streak, 1);
        h.state.worker_acknowledgement = Some(std::sync::Arc::new(
            std::sync::atomic::AtomicBool::new(acknowledged),
        ));
        h.io.tick += 10;
        let next = h.call(room_monitor::Action::Sample)?;
        assert_eq!(next["result"]["ok"], true, "{next}");
        if acknowledged {
            assert_eq!(h.progress()?.phase, "satisfied");
            assert_eq!(h.progress()?.streak, 2);
        } else {
            assert_eq!(h.progress()?.phase, "candidate");
            assert!(!h.progress()?.terminal());
            assert_eq!(h.progress()?.streak, 1);
            assert_eq!(room_result(&next)["sampled_condition_satisfied"], false);
            h.io.tick += 10;
            assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
            assert_eq!(h.progress()?.phase, "satisfied");
        }
    }
    Ok(())
}

#[test]
fn joint_room_source_replacement_is_terminal_and_never_projects_a_fresh_positive_capture()
-> Result<()> {
    let mut h = setup_room(2)?;
    assert_eq!(h.start()?["result"]["ok"], true);
    assert_eq!(h.call(room_monitor::Action::Sample)?["result"]["ok"], true);
    let original_source = h
        .monitor()?
        .store
        .first_source()
        .cloned()
        .ok_or_else(exhausted)?;
    h.io.tick += 10;
    let mut replaced = room_sample(h.definition()?, h.io.tick, true, TerrainFault::None)?;
    replaced.before.generation += 1;
    replaced.after.generation += 1;
    h.io.queued.push_back(replaced);
    let invalidated = h.call(room_monitor::Action::Sample)?;
    assert_eq!(invalidated["result"]["ok"], true, "{invalidated}");
    assert_eq!(h.progress()?.phase, "invalidated");
    assert_eq!(
        room_result(&invalidated)["sampled_condition_satisfied"],
        false
    );
    assert_eq!(
        room_result(&invalidated)["fresh_joint_sample_acquired"],
        false
    );
    assert_eq!(h.monitor()?.store.first_source(), Some(&original_source));
    h.reopen()?;
    assert_eq!(h.progress()?.phase, "invalidated");
    assert_eq!(h.monitor()?.store.first_source(), Some(&original_source));
    Ok(())
}

#[test]
fn legacy_furniture_batch_cannot_acquire_retrospective_room_geometry() -> Result<()> {
    let mut h = setup_clock(2)?;
    h.config.room_completion_path = Some(h.directory.file("room-completion"));
    let c = h.state.context(false, None)?;
    let mut io = JointIo::default();
    let output = room_monitor::run(
        &mut h.state,
        &h.config,
        c,
        "fortress.query",
        room_monitor::Action::Start(room_timing()),
        Instant::now(),
        &mut io,
    );
    let refused: Value = serde_json::from_str(&output).map_err(|_| exhausted())?;
    assert_eq!(refused["result"]["ok"], false, "{refused}");
    assert_eq!(io.samples, 0);
    assert!(h.state.room_completion.is_none());
    assert!(
        !h.config
            .room_completion_path
            .as_ref()
            .ok_or_else(exhausted)?
            .exists()
    );
    Ok(())
}

#[test]
fn final_local_room_cancel_rechecks_and_redacts_lost_origin_or_clock_custody() -> Result<()> {
    for lose_clock in [false, true] {
        // The last callback is the publication boundary. Derive its position
        // from the same successful lifecycle instead of fixing an incidental
        // number of internal preflight callbacks in this regression.
        let last_checkpoint = {
            let mut baseline = setup_room(2)?;
            assert_eq!(baseline.start()?["result"]["ok"], true);
            let _clock = if lose_clock {
                Some(prepare_original_furniture_clock(&mut baseline)?)
            } else {
                None
            };
            let before = baseline.io.checkpoints;
            let result = baseline.call(room_monitor::Action::Cancel)?;
            assert_eq!(result["result"]["ok"], true, "{result}");
            assert_eq!(room_result(&result)["origin_verified"], true);
            baseline.io.checkpoints - before
        };
        let mut h = setup_room(2)?;
        assert_eq!(h.start()?["result"]["ok"], true);
        let _clock = if lose_clock {
            Some(prepare_original_furniture_clock(&mut h)?)
        } else {
            None
        };
        h.io.fault_on_checkpoint = Some(h.io.checkpoints + last_checkpoint);
        h.io.checkpoint_fault = Some(if lose_clock {
            CheckpointFault::Corrupt(h.config.clock.as_ref().ok_or_else(exhausted)?.path.clone())
        } else {
            CheckpointFault::Remove(h.config.batch_path.clone().ok_or_else(exhausted)?)
        });
        let cancelled = h.call(room_monitor::Action::Cancel)?;
        assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
        assert!(
            h.io.checkpoint_fault.is_none(),
            "the final callback must execute"
        );
        assert_eq!(h.progress()?.phase, "cancelled");
        assert_eq!(room_result(&cancelled)["monitor_terminal"], true);
        assert_eq!(room_result(&cancelled)["monitor_inventory_verified"], true);
        assert_eq!(
            room_result(&cancelled)["sampled_condition_satisfied"],
            false
        );
        assert_eq!(cancelled["result"]["game_mutation_dispatched"], false);
        if lose_clock {
            assert_eq!(
                cancelled["result"]["construction_clock"]["history_verified"],
                false
            );
            assert!(!h.state.clock_verified);
        } else {
            assert_eq!(room_result(&cancelled)["origin_verified"], false);
            assert_eq!(cancelled["result"]["batch"]["inventory_verified"], false);
        }
        assert_eq!(
            h.io.samples, 0,
            "local cancellation has no source acquisition"
        );
    }
    Ok(())
}

#[test]
fn offline_room_inspection_and_closed_session_guidance_never_advertise_unavailable_actions()
-> Result<()> {
    let mut h = setup_room(2)?;
    assert_eq!(h.start()?["result"]["ok"], true);
    let mut h = h.into_offline()?;
    assert_eq!(h.state.control.mode(), BuildMode::Offline);
    assert!(
        h.state
            .grants
            .iter()
            .all(|grant| grant.capability == Capability::Query)
    );
    let path = h
        .config
        .room_completion_path
        .as_ref()
        .ok_or_else(exhausted)?
        .clone();
    let retained = fs::read(&path).map_err(storage_error)?;
    let inspected = h.call(room_monitor::Action::Inspect)?;
    assert_eq!(inspected["result"]["ok"], true, "{inspected}");
    let recommendations = inspected["agent_turn"]["recommendations"]
        .as_array()
        .ok_or_else(exhausted)?;
    assert!(!recommendations.is_empty());
    assert!(
        recommendations
            .iter()
            .all(|recommendation| recommendation["tool"] == "fortress.query")
    );
    assert!(inspected["agent_turn"]["active_work"]["room_completion"]["cancellation"].is_null());
    for action in [room_monitor::Action::Sample, room_monitor::Action::Cancel] {
        let refused = h.call(action)?;
        assert_eq!(refused["result"]["ok"], false, "{refused}");
        assert_eq!(fs::read(&path).map_err(storage_error)?, retained);
    }
    assert_eq!(h.io.samples, 0);
    let c = h.state.context(false, None)?;
    let view = h.state.control.inventory(&c)?;
    let closed: Value = serde_json::from_str(&h.state.close_packet(&c, Some(&view), true))
        .map_err(|_| exhausted())?;
    assert_eq!(closed["result"]["closed"], true);
    assert!(
        closed["agent_turn"]["recommendations"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert!(closed["agent_turn"]["active_work"]["room_completion"]["cancellation"].is_null());
    Ok(())
}

#[test]
fn full_capture_room_history_reopens_with_all_twenty_original_samples_and_fixed_limits()
-> Result<()> {
    let mut h = setup_room_with_padding(2, true)?;
    assert_eq!(h.start()?["result"]["ok"], true);
    assert_eq!(
        h.definition()?
            .room()
            .terrain()
            .region()
            .volume()
            .map_err(|_| exhausted())?,
        4096
    );
    let definition = h.definition()?.canonical_bytes().to_vec();
    let timing = h.definition()?.goal().timing();
    h.io.complete = false;
    for sample in 0..20 {
        h.io.tick = TICK + 10 * (sample + 1);
        let pending = h.call(room_monitor::Action::Sample)?;
        assert_eq!(pending["result"]["ok"], true, "sample {sample}: {pending}");
        assert!(!h.progress()?.terminal());
        assert_eq!(h.progress()?.streak, 0);
        assert_eq!(h.progress()?.observations, sample as u32 + 1);
        assert_eq!(room_result(&pending)["sampled_condition_satisfied"], false);
    }
    let bytes = h.monitor()?.store.byte_len();
    assert!(
        bytes > 2 * 1024 * 1024,
        "test must exercise a substantial complete history: {bytes}"
    );
    let recovery = h.monitor()?.store.recovery();
    let source = h.monitor()?.store.first_source().cloned();
    h.reopen()?;
    assert_eq!(h.definition()?.canonical_bytes(), definition);
    assert_eq!(h.definition()?.goal().timing(), timing);
    assert_eq!(h.monitor()?.store.recovery(), recovery);
    assert_eq!(h.monitor()?.store.byte_len(), bytes);
    assert_eq!(h.monitor()?.store.first_source(), source.as_ref());
    assert_eq!(h.progress()?.observations, 20);
    assert!(h.monitor()?.store.authority_tick_floor() >= TICK + 200);
    assert_eq!(h.io.samples, 20, "reopen cannot contact the native source");
    let inspected = h.call(room_monitor::Action::Inspect)?;
    assert_eq!(inspected["result"]["ok"], true, "{inspected}");
    assert_eq!(room_result(&inspected)["observations"], 20);
    assert_eq!(room_result(&inspected)["timing"]["max_observations"], 32);
    assert_eq!(room_result(&inspected)["timing"]["deadline"], TICK + 1000);
    assert_eq!(
        room_result(&inspected)["fresh_joint_sample_acquired"],
        false
    );
    Ok(())
}
