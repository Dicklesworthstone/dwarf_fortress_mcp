#![cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
//! Whole-furniture clock tests use real private journals and independent native
//! fixture records. Injected native calls are counted; no live game is implied.
use super::*;
use dfmcp_adapter::bounded_run::journal::{RunBinding, RunMode};
use dfmcp_adapter::bounded_run::private_file::{PrivateRunFile, open_private_run_journal_exact};
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunSource};
use dfmcp_adapter::bounded_run::{RunObservation, RunPlan, RunRecord};
use dfmcp_adapter::build_placement::journal::private_file::open_private_build_exact;
use dfmcp_adapter::construction_plan::clock::{Clock, Recovery};
use dfmcp_adapter::construction_plan::origin::{MonitorDefinition, Origin};
use dfmcp_adapter::construction_plan::store::open_private_monitor;
use dfmcp_adapter::construction_plan::{Goal, LinkedSample, Manifest, Progress, Timing};
use dfmcp_adapter::live_jobs::LiveJob;
use dfmcp_adapter::live_operations::{LiveOperationsObservation, OperationsProfile};
use std::collections::VecDeque;
use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[path = "clock/opening_tests.rs"]
mod opening_tests;

const TICK: u64 = 806_500;
static DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-furniture-clock-mcp-{}-{}",
            std::process::id(),
            DIRECTORY_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(storage_error)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(storage_error)?;
        Ok(Self(path))
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn storage_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "furniture clock test storage failed",
    )
}
fn missing_fixture() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::InternalInvariantViolation,
        "construction clock fixture missing required state",
    )
}
fn hex(raw: &str) -> Result<Vec<u8>> {
    let raw = raw.trim();
    if raw.len() % 2 != 0 {
        return Err(exhausted());
    }
    raw.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).map_err(|_| exhausted())?, 16)
                .map_err(|_| exhausted())
        })
        .collect()
}
fn original_goal(count: usize) -> Result<Goal> {
    let full = Goal::decode(&hex(include_str!(
        "../../../dfmcp-adapter/src/construction_plan/testdata/python_goal_32.hex"
    ))?)?;
    Goal::new(full.records()[..count].to_vec(), full.timing())
}
fn field16(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
}
fn field32(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u32).to_be_bytes());
    out.extend_from_slice(value);
}
fn domain_hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut all = domain.to_vec();
    all.push(0);
    all.extend_from_slice(bytes);
    Digest32::of_bytes(&all)
}
fn rekey(original: &BuildRecord, key: &str, placed: bool) -> Result<BuildRecord> {
    let plan = BuildPlan::new(key, original.plan().before().clone())?;
    let mut bytes = b"DFMBR019".to_vec();
    field16(&mut bytes, key.as_bytes());
    field16(&mut bytes, plan.before().canonical_bytes());
    bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.token());
    bytes.extend_from_slice(if placed { &[2, 0, 1, 1] } else { &[0, 0, 0, 0] });
    if placed {
        field16(
            &mut bytes,
            original.after().ok_or_else(exhausted)?.canonical_bytes(),
        );
        field16(
            &mut bytes,
            original
                .insertion()
                .ok_or_else(exhausted)?
                .canonical_bytes(),
        );
    }
    let receipt = domain_hash(b"dfmcp-build-receipt/1", &bytes);
    bytes.extend_from_slice(receipt.as_bytes());
    BuildRecord::decode(&bytes)
}
fn append_placement(
    raw: &mut Vec<u8>,
    head: &mut Digest32,
    sequence: &mut u64,
    plan: &BuildPlan,
    state: u8,
    native: Option<&BuildRecord>,
) {
    let mut body = vec![state, u8::from(matches!(state, 2 | 5)), 0];
    field32(&mut body, &plan.canonical_bytes());
    field32(&mut body, native.map_or(&[], BuildRecord::canonical_bytes));
    let mut frame = b"DFMBJF19".to_vec();
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    *sequence += 1;
    frame.extend_from_slice(&sequence.to_be_bytes());
    frame.extend_from_slice(head.as_bytes());
    frame.extend_from_slice(&body);
    *head = domain_hash(b"dfmcp-build-journal-frame/1", &frame);
    frame.extend_from_slice(head.as_bytes());
    frame.extend_from_slice(b"DFMBJEND");
    raw.extend_from_slice(&frame);
}
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).map_err(storage_error)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(storage_error)
}

/// These tests start after all original placements are durably acknowledged.
/// Any accidental placement connection or mutation is a test failure.
struct NoPlacement {
    binding: BuildBinding,
    summary: BuildNativeSummary,
}
impl BuildSource for NoPlacement {
    fn binding(&self) -> &BuildBinding {
        &self.binding
    }
    fn native_summary(&self) -> BuildNativeSummary {
        self.summary
    }
    fn fence(&mut self) {}
    fn observe(
        &mut self,
        _: BuildSelection,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildCapture> {
        Err(error(
            ErrorCode::InternalInvariantViolation,
            "clock path contacted placement observation",
        ))
    }
    fn prepare(
        &mut self,
        _: &BuildPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildPreparation> {
        Err(error(
            ErrorCode::InternalInvariantViolation,
            "clock path prepared placement",
        ))
    }
    fn commit(
        &mut self,
        _: BuildDispatch<'_>,
        _: &OperationContext,
        _: Duration,
    ) -> Result<BuildRecord> {
        Err(error(
            ErrorCode::InternalInvariantViolation,
            "clock path dispatched placement",
        ))
    }
    fn query(
        &mut self,
        _: &BuildPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<Option<BuildRecord>> {
        Err(error(
            ErrorCode::InternalInvariantViolation,
            "clock path reconnected placement source",
        ))
    }
    fn cancel(&mut self, _: &BuildPlan, _: &OperationContext, _: Duration) -> Result<BuildRecord> {
        Err(error(
            ErrorCode::InternalInvariantViolation,
            "clock cancellation retired a placement",
        ))
    }
}
type ClockState = State<PrivateBuildFile, NoPlacement>;

fn linked_sample(
    definition: &MonitorDefinition,
    tick: u64,
    complete: bool,
) -> Result<LinkedSample> {
    let corpus = LinkedSample::decode(&hex(include_str!(
        "../../../dfmcp-adapter/src/construction_plan/testdata/python_sample_32.hex"
    ))?)?;
    let mut observation = LiveOperationsObservation::decode_profile(
        &corpus.capture,
        corpus.operations.generation,
        corpus.operations.df_version.clone(),
        corpus.operations.dfhack_version.clone(),
        OperationsProfile::PagedV1_4,
    )?;
    observation.jobs.year = (tick / 403_200) as u32;
    observation.jobs.year_tick = (tick % 403_200) as u32;
    observation.jobs.paused = true;
    let original = definition.origin().receipts();
    observation.buildings.retain(|building| {
        original.iter().any(|record| {
            record
                .insertion()
                .is_some_and(|v| v.building_id() == building.native_id)
        })
    });
    observation.items.retain(|item| {
        original.iter().any(|record| {
            record
                .insertion()
                .is_some_and(|v| v.item_id() == item.native_id)
        })
    });
    if !complete {
        for building in &mut observation.buildings {
            building.build_stage = 0;
        }
        for record in original {
            let insertion = record.insertion().ok_or_else(exhausted)?;
            let [x, y, z] = insertion.position();
            observation.jobs.jobs.push(LiveJob {
                native_id: insertion.job_id(),
                job_type: 1,
                type_key: "ConstructBuilding".to_owned(),
                reaction: String::new(),
                suspended: false,
                repeating: false,
                position: MapCoord::new(x as i32, y as i32, z as i32),
                worker_native_id: None,
                holder_native_id: Some(insertion.building_id()),
                completion_timer: -1,
                attached_item_count: 0,
                required_item_filter_count: 0,
            });
        }
    }
    let records = original
        .iter()
        .map(|record| record.canonical_bytes().to_vec())
        .collect::<Vec<_>>();
    let manifest = Manifest {
        generation: original[0].plan().before().generation(),
        df_version: corpus.before.df_version,
        dfhack_version: corpus.before.dfhack_version,
    };
    Ok(LinkedSample {
        before: manifest.clone(),
        before_records: records.clone(),
        operations: corpus.operations,
        capture: observation.encode_profile(OperationsProfile::PagedV1_4)?,
        after: manifest,
        after_records: records,
    })
}

fn run_observation(
    generation: u64,
    sequence: u64,
    tick: u64,
    paused: bool,
) -> Result<RunObservation> {
    let mut bytes = b"DFMRO013".to_vec();
    for value in [generation, sequence, tick] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(&[1, 1, u8::from(paused)]);
    RunObservation::decode(&bytes)
}
fn run_record(plan: &RunPlan, phase: u8, reason: u8, tick: u64) -> Result<RunRecord> {
    let mut bytes = b"DFMRE013".to_vec();
    field16(&mut bytes, plan.key().as_bytes());
    bytes.extend_from_slice(&plan.spec().game_ticks().to_be_bytes());
    bytes.extend_from_slice(&plan.spec().wall_ms().to_be_bytes());
    bytes.extend_from_slice(plan.before().canonical_bytes());
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
    let digest = domain_hash(b"dfmcp-bounded-run-receipt/1", &bytes);
    bytes.extend_from_slice(digest.as_bytes());
    RunRecord::decode(&bytes)
}
struct RunGame {
    tick: u64,
    sequence: u64,
    paused: bool,
    record: Option<RunRecord>,
    calls: Vec<&'static str>,
    lose_commit: bool,
    absent: bool,
}
impl RunGame {
    fn new() -> Self {
        Self {
            tick: TICK + 1,
            sequence: 1,
            paused: true,
            record: None,
            calls: Vec::new(),
            lose_commit: false,
            absent: false,
        }
    }
    fn finish(&mut self) -> Result<()> {
        let plan = self.record.as_ref().ok_or_else(exhausted)?.plan().clone();
        self.tick =
            plan.before().tick().ok_or_else(exhausted)? + u64::from(plan.spec().game_ticks());
        self.paused = true;
        self.record = Some(run_record(&plan, 3, 1, self.tick)?);
        Ok(())
    }
}
struct RunNative {
    manifest: RunManifest,
    game: Rc<RefCell<RunGame>>,
}
impl RunSource for RunNative {
    fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(SocketAddr::from(([127, 0, 0, 1], 5000)))
    }
    fn observe(&mut self, _: &OperationContext) -> Result<RunObservation> {
        let mut game = self.game.borrow_mut();
        game.calls.push("observe");
        run_observation(
            self.manifest.generation,
            game.sequence,
            game.tick,
            game.paused,
        )
    }
    fn prepare(&mut self, plan: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        let mut game = self.game.borrow_mut();
        game.calls.push("prepare");
        let record = run_record(plan, 0, 0, 0)?;
        game.record = Some(record.clone());
        Ok(record)
    }
    fn commit(&mut self, plan: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        let mut game = self.game.borrow_mut();
        game.calls.push("commit");
        game.sequence += 1;
        game.paused = false;
        let record = run_record(plan, 1, 0, game.tick)?;
        game.record = Some(record.clone());
        if game.lose_commit {
            Err(error(
                ErrorCode::AdapterUnavailable,
                "lost clock commit reply",
            ))
        } else {
            Ok(record)
        }
    }
    fn query(&mut self, plan: &RunPlan, _: &OperationContext) -> Result<Option<RunRecord>> {
        let mut game = self.game.borrow_mut();
        game.calls.push("query");
        if let Some(record) = &game.record {
            assert_eq!(record.plan(), plan);
        }
        Ok(if game.absent {
            None
        } else {
            game.record.clone()
        })
    }
    fn cancel(&mut self, plan: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        let mut game = self.game.borrow_mut();
        game.calls.push("cancel");
        let prepared = game
            .record
            .as_ref()
            .is_some_and(|record| record.phase() == dfmcp_adapter::bounded_run::RunPhase::Prepared);
        game.paused = true;
        let record = run_record(plan, if prepared { 4 } else { 3 }, 3, game.tick)?;
        game.record = Some(record.clone());
        Ok(record)
    }
}

struct ClockIo {
    game: Rc<RefCell<RunGame>>,
    manifest: RunManifest,
    samples: VecDeque<LinkedSample>,
    complete: bool,
    acquisitions: usize,
    checkpoints: usize,
    deny_control: bool,
    lose_parent_after_sample: Option<PathBuf>,
    opening: Option<Recovery>,
}
impl clock::ClockIo for ClockIo {
    type Source = RunNative;

    fn opening_recovery(&self) -> Option<Recovery> {
        self.opening
    }
    fn open(
        &mut self,
        config: &clock::Config,
        definition: &MonitorDefinition,
        operations: &Manifest,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<Clock<PrivateRunFile>> {
        self.opening = config.recovery.or_else(|| {
            let id = config
                .binding
                .initial_journal_id(n.session_id, n.request_id);
            Some(Recovery {
                journal_id: id,
                head: id,
            })
        });
        let journal = open_private_run_journal_exact(
            &config.path,
            n,
            RunMode::Control,
            Some(config.binding.clone()),
            config.initialize,
        )?;
        let owner = Clock::new(
            journal,
            definition.clone(),
            operations.clone(),
            config.recovery,
            c,
            n,
        )?;
        self.opening = Some(owner.recovery());
        Ok(owner)
    }
    fn connect(
        &mut self,
        config: &clock::Config,
        _: &OperationContext,
        control: bool,
    ) -> Result<Self::Source> {
        if control && self.deny_control {
            return Err(error(ErrorCode::CapabilityDenied, "clock opt-in withdrawn"));
        }
        assert_eq!(config.binding.manifest(), &self.manifest);
        Ok(RunNative {
            manifest: self.manifest.clone(),
            game: self.game.clone(),
        })
    }
    fn sample(
        &mut self,
        definition: &MonitorDefinition,
        _: &OperationContext,
    ) -> Result<LinkedSample> {
        self.acquisitions += 1;
        let sample = if let Some(sample) = self.samples.pop_front() {
            sample
        } else {
            linked_sample(definition, self.game.borrow().tick, self.complete)?
        };
        if let Some(path) = self.lose_parent_after_sample.take() {
            fs::remove_file(path).map_err(storage_error)?;
        }
        Ok(sample)
    }
    fn checkpoint(&mut self, _: &OperationContext, control: bool) -> Result<()> {
        self.checkpoints += 1;
        if control && self.deny_control {
            Err(error(ErrorCode::CapabilityDenied, "clock opt-in withdrawn"))
        } else {
            Ok(())
        }
    }
}

struct Harness {
    state: ClockState,
    config: Config,
    io: ClockIo,
    directory: Directory,
}
fn clock_grants() -> Vec<CapabilityGrant> {
    [
        Capability::Query,
        Capability::Plan,
        Capability::ControlClock,
    ]
    .into_iter()
    .map(|capability| CapabilityGrant {
        capability,
        scope: CapabilityScope {
            fortress_id: Some(dfmcp_core::FortressId::NIL),
            ..CapabilityScope::default()
        },
        max_risk: if capability == Capability::Query {
            RiskTier::ReadOnly
        } else {
            RiskTier::Guarded
        },
        expires_at_tick: None,
        remaining_uses: None,
    })
    .collect()
}
fn setup_clock(count: usize) -> Result<Harness> {
    setup_clock_with_observations(count, 32)
}
fn setup_clock_with_observations(count: usize, max_observations: u32) -> Result<Harness> {
    let directory = Directory::new()?;
    let originals = original_goal(count)?;
    let before = originals.records()[0].plan().before();
    let endpoint = SocketAddr::from(([127, 0, 0, 1], 5000));
    let binding = BuildBinding::new(endpoint, "test-df", "test-dfhack", before)?;
    let mut config = config()?;
    config.path = directory.file("placements");
    config.batch_path = Some(directory.file("batch"));
    config.completion_path = Some(directory.file("completion"));
    config.fortress = before.fortress().clone();
    config.clock = None;
    let mut work_budget = budget();
    work_budget.max_game_ticks = 1200;
    work_budget.max_entities = 80_000;
    let mut c = context(
        SessionId::new(800 + count as u128),
        RequestId::new(1),
        before.fortress_id(),
        TICK + 1,
        work_budget,
        BuildMode::Control,
        true,
    );
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
    let mut raw = b"DFMBJ019".to_vec();
    field32(&mut raw, &binding.canonical_bytes());
    raw.extend_from_slice(&[3; 32]);
    let journal_id = domain_hash(b"dfmcp-build-journal/1", &raw);
    raw.extend_from_slice(journal_id.as_bytes());
    let steps = originals
        .records()
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let s = record.plan().before().selection();
            json!({"name":format!("s{index:02}"),"kind":s.kind().as_str(),
                "item":s.item_id(),"target":s.target(),
                "after":if index==0 {Vec::<String>::new()} else {vec![format!("s{:02}", index-1)]}})
        })
        .collect::<Vec<_>>();
    let plan = FurniturePlan::decode(
        json!({"schema":"dfmcp.furniture-plan/1","steps":steps})
            .to_string()
            .as_bytes(),
    )?;
    let batch = BatchDefinition::new(plan, binding.clone(), journal_id)?;
    let mut head = journal_id;
    let mut sequence = 0;
    let mut placed = Vec::new();
    for (step, record) in batch.plan().ordered_steps().zip(originals.records()) {
        let key = batch.key(step);
        let prepared = rekey(record, &key, false)?;
        let complete = rekey(record, &key, true)?;
        for (state, native) in [
            (0, None),
            (1, Some(&prepared)),
            (2, Some(&prepared)),
            (5, Some(&complete)),
        ] {
            append_placement(
                &mut raw,
                &mut head,
                &mut sequence,
                complete.plan(),
                state,
                native,
            );
        }
        placed.push(complete);
    }
    write_private(&config.path, &raw)?;
    let journal = open_private_build_exact(
        &config.path,
        &c,
        BuildMode::Control,
        Some(binding.clone()),
        false,
    )?;
    let mut state = State::new(BuildSession::new(journal, &c)?, &c, &config)?;
    let inventory = state.control.inventory(&c)?;
    let parent = open_private_batch(
        config.batch_path.as_ref().ok_or_else(exhausted)?,
        &c,
        BuildMode::Control,
        Some(batch.clone()),
    )?;
    let origin = Origin::from_batch(
        batch,
        &binding,
        &inventory,
        parent.private_identity(&c)?,
        state.control.private_identity(&c)?,
    )?;
    let goal = Goal::new(
        placed,
        Timing {
            deadline: TICK + 1000,
            interval: 1,
            stable_samples: 2,
            stable_span: 1,
            max_gap: 1200,
            max_observations,
        },
    )?;
    let definition = MonitorDefinition::new(origin, goal)?;
    let first_sample = linked_sample(&definition, TICK + 1, false)?;
    let mut store = open_private_monitor(
        config.completion_path.as_ref().ok_or_else(exhausted)?,
        &c,
        BuildMode::Control,
        Some(definition.clone()),
    )?;
    let mut admit = |_: &MonitorDefinition, _: &Progress| Ok(());
    let mut guard = |_: &Origin, _: &OperationContext| Ok(());
    store.begin_read(&c, &mut admit, &mut guard)?;
    store.publish_sample(&c, &first_sample, &mut admit, &mut guard)?;
    assert_eq!(store.progress().phase, "active");
    assert_eq!(store.progress().assessments.len(), count);
    drop(store);
    state.batch = Some(parent);
    completion::reopen(
        &mut state,
        &config,
        &c,
        &mut Work::new(&c, Instant::now())?,
        &inventory,
    )?;
    let manifest = RunManifest {
        generation: 41,
        df_version: "test-df".into(),
        dfhack_version: "test-dfhack".into(),
    };
    let clock_config = clock::Config {
        path: directory.file("clock"),
        initialize: true,
        binding: RunBinding::new(endpoint, manifest.clone())?,
        recovery: None,
    };
    let mut io = ClockIo {
        game: Rc::new(RefCell::new(RunGame::new())),
        manifest,
        samples: VecDeque::new(),
        complete: false,
        acquisitions: 0,
        checkpoints: 0,
        deny_control: false,
        lose_parent_after_sample: None,
        opening: None,
    };
    state.clock_grants = clock_grants();
    let n = clock::native_context(&c, &state.clock_grants)?;
    let owner = clock::ClockIo::open(
        &mut io,
        &clock_config,
        &definition,
        &first_sample.operations,
        &c,
        &n,
    )?;
    state.clock_recovery = Some(owner.recovery());
    state.clock = Some(owner);
    state.clock_config = Some(clock_config.clone());
    state.authority_tick = TICK + 1;
    config.clock = Some(clock_config);
    Ok(Harness {
        state,
        config,
        io,
        directory,
    })
}
impl Harness {
    fn id(&self) -> Result<Digest32> {
        Ok(self
            .state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .definition()
            .id())
    }
    fn call(&mut self, action: clock::Action) -> Result<Value> {
        let op = match action {
            clock::Action::Prepare(_) => "fortress.plan",
            clock::Action::Commit { .. } => "fortress.commit",
            clock::Action::Wait(_) => "fortress.wait",
            clock::Action::Cancel(_) => "fortress.cancel",
            clock::Action::Inspect { .. } => "fortress.query",
        };
        let c = self.state.context(true, None)?;
        let raw = clock::run_action(&mut self.state, c, op, action, Instant::now(), &mut self.io);
        assert!(raw.len() as u64 <= OUTPUT_BYTES);
        serde_json::from_str(&raw).map_err(|_| exhausted())
    }
    fn prepare(&mut self) -> Result<Value> {
        self.call(clock::Action::Prepare(clock::Request {
            monitor_id: self.id()?,
            game_ticks: 10,
            wall_millis: 5000,
        }))
    }
    fn commit(&mut self, seal: Digest32) -> Result<Value> {
        self.call(clock::Action::Commit {
            id: self.id()?,
            seal,
        })
    }
    fn seal(&self) -> Result<Digest32> {
        self.state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .prepared_seal()
            .ok_or_else(missing_fixture)
    }
}

#[test]
fn whole_furniture_clock_separates_review_dispatch_reconciliation_and_completion() -> Result<()> {
    let mut h = setup_clock(3)?;
    let original_id = h.id()?;
    let original = h
        .state
        .completion
        .as_ref()
        .ok_or_else(exhausted)?
        .store
        .progress()
        .clone();
    let prepared = h.prepare()?;
    assert_eq!(prepared["result"]["ok"], true, "{prepared}");
    assert_eq!(prepared["result"]["confirmation_required"], true);
    assert_eq!(prepared["result"]["goal_sample_published"], false);
    assert_eq!(prepared["result"]["batch"]["total"], 3);
    assert_eq!(
        prepared["result"]["completion"]["assessments"]
            .as_array()
            .map(Vec::len),
        Some(3)
    );
    let clock = &prepared["result"]["construction_clock"];
    assert_eq!(clock["monitor_id"], original_id.to_string());
    assert_eq!(clock["readiness"]["pending_count"], 3);
    assert_eq!(clock["readiness"]["goal_sample_published"], false);
    assert_eq!(clock["pending"], true);
    assert_eq!(clock["fresh_goal_sampling_permitted"], false);
    assert_eq!(h.io.game.borrow().calls, ["observe", "observe", "prepare"]);
    let readiness_progress = h
        .state
        .completion
        .as_ref()
        .ok_or_else(missing_fixture)?
        .store
        .progress();
    assert_eq!(readiness_progress.goal_digest, original.goal_digest);
    assert_eq!(readiness_progress.observations, original.observations + 1);
    assert_eq!(readiness_progress.streak, 0);
    assert!(!readiness_progress.reading);
    assert_eq!(prepared["result"]["readiness_acquisition_retained"], true);
    assert_eq!(
        prepared["result"]["readiness_earns_stability_credit"],
        false
    );
    let c = h.state.context(false, None)?;
    assert!(clock::ensure_for_sample(&mut h.state, &c).is_err());
    let seal = h.seal()?;
    let committed = h.commit(seal)?;
    assert_eq!(committed["result"]["ok"], true, "{committed}");
    assert_eq!(committed["result"]["game_unpause_requested"], true);
    assert_eq!(
        committed["result"]["construction_clock"]["may_be_running"],
        true
    );
    assert_eq!(h.io.acquisitions, 2);
    assert_eq!(
        h.io.game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "commit")
            .count(),
        1
    );
    assert_eq!(
        h.state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .progress()
            .observations,
        3
    );
    assert!(
        h.state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .prepared_seal()
            .is_none()
    );
    let waiting = h.call(clock::Action::Wait(original_id))?;
    assert_eq!(waiting["result"]["ok"], true, "{waiting}");
    assert_eq!(waiting["result"]["construction_clock"]["pending"], true);
    h.io.game.borrow_mut().finish()?;
    let settled = h.call(clock::Action::Wait(original_id))?;
    assert_eq!(settled["result"]["ok"], true, "{settled}");
    assert_eq!(settled["result"]["construction_clock"]["pending"], false);
    assert_eq!(
        settled["result"]["construction_clock"]["current_pause_proven"],
        false
    );
    assert_eq!(
        settled["result"]["construction_clock"]["sampled_construction_condition_proven"],
        false
    );
    assert_eq!(
        h.state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .progress()
            .observations,
        3
    );
    let c = h.state.context(false, None)?;
    clock::ensure_for_sample(&mut h.state, &c)?;
    assert_eq!(h.id()?, original_id);
    assert_eq!(
        h.io.game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "commit")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn clock_requires_separate_native_grants_and_never_reuses_a_placement_seal() -> Result<()> {
    for canonical in [false, true] {
        let mut h = setup_clock(3)?;
        if canonical {
            h.state
                .grants
                .retain(|grant| grant.capability != Capability::ControlClock);
        } else {
            h.state
                .clock_grants
                .retain(|grant| grant.capability != Capability::ControlClock);
        }
        let denied = h.prepare()?;
        assert_eq!(denied["result"]["ok"], false, "{denied}");
        assert!(h.io.game.borrow().calls.is_empty());
        assert_eq!(h.io.acquisitions, 0);
    }
    let mut h = setup_clock(3)?;
    assert_eq!(h.prepare()?["result"]["ok"], true);
    let wrong = Digest32::of_bytes(b"a placement review never authorizes a clock interval");
    let denied = h.commit(wrong)?;
    assert_eq!(denied["result"]["ok"], false);
    assert_eq!(denied["result"]["construction_clock"]["pending"], true);
    assert!(!h.io.game.borrow().calls.contains(&"commit"));
    assert!(
        h.state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .prepared_seal()
            .is_none()
    );
    let c = h.state.context(false, None)?;
    assert!(clock::ensure_for_sample(&mut h.state, &c).is_err());
    Ok(())
}

#[test]
fn readiness_rejects_a_missing_original_member_and_consumes_newer_tick_on_failure() -> Result<()> {
    let mut missing = setup_clock(3)?;
    let definition = missing
        .state
        .completion
        .as_ref()
        .ok_or_else(missing_fixture)?
        .store
        .definition()
        .clone();
    let mut sample = linked_sample(&definition, TICK + 1, false)?;
    sample.after_records.pop();
    missing.io.samples.push_back(sample);
    let denied = missing.prepare()?;
    assert_eq!(denied["result"]["ok"], false, "{denied}");
    assert!(!missing.io.game.borrow().calls.contains(&"prepare"));
    assert!(
        missing
            .state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .cached_view()
            .pending
    );
    assert_eq!(
        missing
            .state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .progress()
            .observations,
        1
    );

    let mut later = setup_clock(3)?;
    let definition = later
        .state
        .completion
        .as_ref()
        .ok_or_else(missing_fixture)?
        .store
        .definition()
        .clone();
    later
        .io
        .samples
        .push_back(linked_sample(&definition, TICK + 20, false)?);
    later.io.lose_parent_after_sample = later.config.batch_path.clone();
    let denied = later.prepare()?;
    assert_eq!(denied["result"]["ok"], false, "{denied}");
    assert!(later.state.authority_tick >= TICK + 20);
    assert!(
        denied["result"]["construction_clock"]["authority_tick_floor"]
            .as_u64()
            .is_some_and(|tick| tick >= TICK + 20),
        "{denied}"
    );
    assert!(!later.io.game.borrow().calls.contains(&"prepare"));
    assert!(
        later
            .state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .prepared_seal()
            .is_none()
    );
    assert!(later.state.context(true, None)?.anchor.tick.get() >= TICK + 20);
    Ok(())
}

#[test]
fn fresh_readiness_runs_again_at_commit_and_parent_loss_stops_unpause() -> Result<()> {
    let mut h = setup_clock(3)?;
    assert_eq!(h.prepare()?["result"]["ok"], true);
    let seal = h.seal()?;
    h.io.lose_parent_after_sample = h.config.batch_path.clone();
    let denied = h.commit(seal)?;
    assert_eq!(denied["result"]["ok"], false, "{denied}");
    assert_eq!(h.io.acquisitions, 2);
    assert!(!h.io.game.borrow().calls.contains(&"commit"));
    assert!(
        h.state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .cached_view()
            .pending
    );
    assert!(
        h.state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .prepared_seal()
            .is_none()
    );
    // Original placement loss cannot erase or strand the retained clock intent.
    let cancelled = h.call(clock::Action::Cancel(h.id()?))?;
    assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
    assert!(!h.io.game.borrow().calls.contains(&"commit"));
    Ok(())
}

#[test]
fn lost_commit_reply_blocks_new_runs_and_sampling_until_original_reconciliation() -> Result<()> {
    let mut h = setup_clock(3)?;
    assert_eq!(h.prepare()?["result"]["ok"], true);
    let seal = h.seal()?;
    h.io.game.borrow_mut().lose_commit = true;
    let lost = h.commit(seal)?;
    assert_eq!(lost["result"]["ok"], false, "{lost}");
    assert_eq!(lost["result"]["effect_may_have_occurred"], true);
    assert_eq!(
        h.io.game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "commit")
            .count(),
        1
    );
    let denied = h.prepare()?;
    assert_eq!(denied["result"]["ok"], false);
    let c = h.state.context(false, None)?;
    assert!(clock::ensure_for_sample(&mut h.state, &c).is_err());
    let retry = h.commit(seal)?;
    assert_eq!(retry["result"]["ok"], false);
    assert_eq!(
        h.io.game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "commit")
            .count(),
        1
    );
    h.io.game.borrow_mut().finish()?;
    let reconciled = h.call(clock::Action::Wait(h.id()?))?;
    assert_eq!(reconciled["result"]["ok"], true, "{reconciled}");
    assert_eq!(reconciled["result"]["construction_clock"]["pending"], false);
    assert_eq!(
        h.state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .progress()
            .observations,
        3
    );
    Ok(())
}

#[test]
fn safety_cancellation_survives_original_file_loss_deadline_and_future_budget_exhaustion()
-> Result<()> {
    let mut h = setup_clock(3)?;
    assert_eq!(h.prepare()?["result"]["ok"], true);
    let seal = h.seal()?;
    assert_eq!(h.commit(seal)?["result"]["ok"], true);
    fs::remove_file(&h.config.path).map_err(storage_error)?;
    fs::remove_file(h.config.batch_path.as_ref().ok_or_else(missing_fixture)?)
        .map_err(storage_error)?;
    h.state.grants.retain(|grant| {
        matches!(
            grant.capability,
            Capability::Query | Capability::ControlClock
        )
    });
    h.state.clock_grants.retain(|grant| {
        matches!(
            grant.capability,
            Capability::Query | Capability::ControlClock
        )
    });
    h.state.budget.max_game_ticks = 0;
    h.state.authority_tick = TICK + 1001;
    h.io.game.borrow_mut().tick = TICK + 1001;
    let cancelled = h.call(clock::Action::Cancel(h.id()?))?;
    assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
    assert_eq!(cancelled["result"]["new_unpause_attempted"], false);
    assert_eq!(cancelled["result"]["construction_clock"]["pending"], false);
    assert_eq!(cancelled["result"]["batch"]["inventory_verified"], false);
    assert_eq!(
        h.io.game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "cancel")
            .count(),
        1
    );
    assert_eq!(
        h.io.game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "commit")
            .count(),
        1
    );
    assert_eq!(h.io.acquisitions, 2);
    Ok(())
}

#[test]
fn absent_native_run_history_never_unlocks_construction_proof_or_another_run() -> Result<()> {
    let mut h = setup_clock(3)?;
    assert_eq!(h.prepare()?["result"]["ok"], true);
    let seal = h.seal()?;
    assert_eq!(h.commit(seal)?["result"]["ok"], true);
    h.io.game.borrow_mut().absent = true;
    let lost = h.call(clock::Action::Wait(h.id()?))?;
    assert_eq!(
        lost["result"]["construction_clock"]["unresolved"], true,
        "{lost}"
    );
    let c = h.state.context(false, None)?;
    assert!(clock::ensure_for_sample(&mut h.state, &c).is_err());
    assert_eq!(h.prepare()?["result"]["ok"], false);
    assert_eq!(
        h.io.game
            .borrow()
            .calls
            .iter()
            .filter(|&&v| v == "commit")
            .count(),
        1
    );
    assert_eq!(
        h.state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .progress()
            .observations,
        3
    );
    Ok(())
}

#[test]
fn clock_query_is_local_head_pinned_and_preserves_full_furniture_identity() -> Result<()> {
    let mut h = setup_clock(3)?;
    assert_eq!(h.prepare()?["result"]["ok"], true);
    let recovery = h
        .state
        .clock
        .as_ref()
        .ok_or_else(missing_fixture)?
        .recovery();
    let calls = h.io.game.borrow().calls.clone();
    let inspected = h.call(clock::Action::Inspect {
        id: h.id()?,
        head: recovery.head,
        offset: 0,
        limit: 1,
    })?;
    assert_eq!(inspected["result"]["ok"], true, "{inspected}");
    assert_eq!(inspected["result"]["native_calls"], 0);
    assert_eq!(
        inspected["result"]["construction_clock"]["runs_page"]["total"],
        1
    );
    assert_eq!(inspected["result"]["batch"]["total"], 3);
    assert_eq!(h.io.game.borrow().calls, calls);
    let stale = h.call(clock::Action::Inspect {
        id: h.id()?,
        head: Digest32::of_bytes(b"stale clock head"),
        offset: 0,
        limit: 1,
    })?;
    assert_eq!(stale["result"]["ok"], false);
    assert_eq!(
        stale["result"]["error"]["code"],
        ErrorCode::StaleAnchor.as_str()
    );
    assert_eq!(h.io.game.borrow().calls, calls);
    Ok(())
}

#[test]
fn reopening_clock_history_retains_exact_pending_run_without_restoring_review_permission()
-> Result<()> {
    let mut h = setup_clock(3)?;
    assert_eq!(h.prepare()?["result"]["ok"], true);
    let old_seal = h.seal()?;
    let recovery = h
        .state
        .clock
        .as_ref()
        .ok_or_else(missing_fixture)?
        .recovery();
    let before_calls = h.io.game.borrow().calls.clone();
    let definition = h
        .state
        .completion
        .as_ref()
        .ok_or_else(missing_fixture)?
        .store
        .definition()
        .clone();
    let operations = h
        .state
        .completion
        .as_ref()
        .ok_or_else(missing_fixture)?
        .store
        .progress()
        .source
        .as_ref()
        .ok_or_else(missing_fixture)?
        .operations
        .clone();
    drop(h.state.clock.take());
    let c = h.state.context(true, None)?;
    let n = clock::native_context(&c, &h.state.clock_grants)?;
    let config = h.state.clock_config.as_mut().ok_or_else(missing_fixture)?;
    // Existing bytes cannot be adopted under a fresh initialization request.
    assert!(clock::ClockIo::open(&mut h.io, config, &definition, &operations, &c, &n).is_err());
    assert!(clock::ensure_for_sample(&mut h.state, &c).is_err());
    let config = h.state.clock_config.as_mut().ok_or_else(missing_fixture)?;
    config.initialize = false;
    config.recovery = Some(recovery);
    let owner = clock::ClockIo::open(&mut h.io, config, &definition, &operations, &c, &n)?;
    assert_eq!(owner.recovery(), recovery);
    assert!(owner.prepared_seal().is_none());
    assert!(owner.cached_view().pending);
    h.state.clock = Some(owner);
    let denied = h.commit(old_seal)?;
    assert_eq!(denied["result"]["ok"], false, "{denied}");
    assert_eq!(h.io.game.borrow().calls, before_calls);
    assert_eq!(
        h.state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .recovery(),
        recovery
    );
    let cancelled = h.call(clock::Action::Cancel(h.id()?))?;
    assert_eq!(cancelled["result"]["ok"], true, "{cancelled}");
    assert_eq!(cancelled["result"]["construction_clock"]["pending"], false);
    assert_eq!(h.io.game.borrow().calls, before_calls);
    Ok(())
}

#[test]
fn original_deadline_tick_budget_checkpoint_and_expiring_clock_grants_gate_new_intervals()
-> Result<()> {
    for case in 0..5 {
        let mut h = setup_clock(3)?;
        match case {
            0 => h.state.budget.max_game_ticks = 0,
            1 => h.state.authority_tick = TICK + 995,
            2 => h.state.policy.config.checkpoint = runtime::CheckpointPolicy::Required,
            3 => {
                for grant in &mut h.state.clock_grants {
                    if grant.capability == Capability::ControlClock {
                        grant.expires_at_tick = Some(GameTick(TICK + 5));
                    }
                }
            }
            _ => h.io.deny_control = true,
        }
        let denied = h.prepare()?;
        assert_eq!(denied["result"]["ok"], false, "case {case}: {denied}");
        assert!(h.io.game.borrow().calls.is_empty(), "case {case}");
        assert_eq!(h.io.acquisitions, 0);
        assert_eq!(
            h.state
                .clock
                .as_ref()
                .ok_or_else(missing_fixture)?
                .cached_view()
                .runs
                .len(),
            0
        );
        assert_eq!(
            h.state
                .completion
                .as_ref()
                .ok_or_else(missing_fixture)?
                .store
                .progress()
                .observations,
            1
        );
    }
    Ok(())
}

fn publish_complete_goal_sample(h: &mut Harness) -> Result<()> {
    let c = h.state.context(false, None)?;
    clock::ensure_for_sample(&mut h.state, &c)?;
    let tick = h.io.game.borrow().tick;
    let monitor = h.state.completion.as_mut().ok_or_else(missing_fixture)?;
    let sample = linked_sample(monitor.store.definition(), tick, true)?;
    let mut admit = |_: &MonitorDefinition, _: &Progress| Ok(());
    let mut guard = |_: &Origin, _: &OperationContext| Ok(());
    monitor.store.begin_read(&c, &mut admit, &mut guard)?;
    monitor
        .store
        .publish_sample(&c, &sample, &mut admit, &mut guard)?;
    Ok(())
}

#[test]
fn whole_plan_stability_survives_two_real_clock_intervals_between_credited_samples() -> Result<()> {
    for count in [2, 32] {
        let mut h = setup_clock(count)?;
        let original_id = h.id()?;
        for interval in 0..3 {
            if interval > 0 {
                h.io.complete = true;
            }
            let before = h
                .state
                .completion
                .as_ref()
                .ok_or_else(missing_fixture)?
                .store
                .progress()
                .observations;
            let prepared = h.prepare()?;
            assert_eq!(prepared["result"]["ok"], true, "{prepared}");
            assert_eq!(prepared["result"]["goal_sample_published"], false);
            assert_eq!(prepared["result"]["readiness_acquisition_retained"], true);
            assert_eq!(
                prepared["result"]["readiness_earns_stability_credit"],
                false
            );
            let progress = h
                .state
                .completion
                .as_ref()
                .ok_or_else(missing_fixture)?
                .store
                .progress();
            assert_eq!(progress.observations, before + 1);
            assert_eq!(progress.streak, u32::from(interval > 0));
            assert!(!progress.reading);
            if interval > 0 {
                assert_eq!(progress.first_tick, Some(TICK + 11));
                assert_eq!(progress.counted_tick, Some(TICK + 11));
            }
            let committed = h.commit(h.seal()?)?;
            assert_eq!(committed["result"]["ok"], true, "{committed}");
            let progress = h
                .state
                .completion
                .as_ref()
                .ok_or_else(missing_fixture)?
                .store
                .progress();
            assert_eq!(progress.observations, before + 2);
            assert_eq!(progress.streak, u32::from(interval > 0));
            h.io.game.borrow_mut().finish()?;
            let settled = h.call(clock::Action::Wait(original_id))?;
            assert_eq!(settled["result"]["ok"], true, "{settled}");
            assert_eq!(settled["result"]["construction_clock"]["pending"], false);
            if interval == 0 {
                publish_complete_goal_sample(&mut h)?;
                let progress = h
                    .state
                    .completion
                    .as_ref()
                    .ok_or_else(missing_fixture)?
                    .store
                    .progress();
                assert_eq!(progress.phase, "candidate");
                assert_eq!((progress.observations, progress.streak), (4, 1));
            }
        }
        assert_eq!(h.io.game.borrow().tick, TICK + 31);
        publish_complete_goal_sample(&mut h)?;
        let progress = h
            .state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .progress();
        assert_eq!(progress.phase, "satisfied");
        assert_eq!((progress.observations, progress.streak), (9, 2));
        assert_eq!(progress.condition_met_count(), count);
        assert_eq!(progress.first_tick, Some(TICK + 11));
        assert_eq!(progress.counted_tick, Some(TICK + 31));
        assert_eq!(h.id()?, original_id);
        assert_eq!(h.io.acquisitions, 6);
        assert_eq!(
            h.io.game
                .borrow()
                .calls
                .iter()
                .filter(|&&call| call == "commit")
                .count(),
            3
        );
        assert_eq!(h.prepare()?["result"]["ok"], false);
    }
    Ok(())
}

#[test]
fn final_original_acquisition_expires_before_prepare_or_commit_unpause() -> Result<()> {
    for maximum in [2, 3] {
        let mut h = setup_clock_with_observations(32, maximum)?;
        let prepared = h.prepare()?;
        let rejected = if maximum == 2 {
            assert!(!h.io.game.borrow().calls.contains(&"prepare"));
            prepared
        } else {
            assert_eq!(prepared["result"]["ok"], true, "{prepared}");
            assert_eq!(
                h.state
                    .completion
                    .as_ref()
                    .ok_or_else(missing_fixture)?
                    .store
                    .progress()
                    .observations,
                2
            );
            h.commit(h.seal()?)?
        };
        assert_eq!(rejected["result"]["ok"], false, "{rejected}");
        assert!(!h.io.game.borrow().calls.contains(&"commit"));
        assert_eq!(h.io.acquisitions, (maximum - 1) as usize);
        let monitor = h.state.completion.take().ok_or_else(missing_fixture)?;
        assert_eq!(monitor.store.progress().phase, "expired");
        assert_eq!(monitor.store.progress().reason, "sample_budget_exhausted");
        assert_eq!(
            (
                monitor.store.progress().observations,
                monitor.store.progress().streak
            ),
            (maximum, 0)
        );
        drop(monitor);
        let c = h.state.context(false, None)?;
        let reopened = open_private_monitor(
            h.config
                .completion_path
                .as_ref()
                .ok_or_else(missing_fixture)?,
            &c,
            BuildMode::Recover,
            None,
        )?;
        assert_eq!(reopened.progress().phase, "expired");
        assert_eq!(reopened.progress().observations, maximum);
    }
    Ok(())
}

#[test]
fn readiness_persists_complete_negative_evidence_before_refusing_the_clock() -> Result<()> {
    for source_change in [false, true] {
        let mut h = setup_clock(32)?;
        let definition = h
            .state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .definition()
            .clone();
        let mut sample = linked_sample(&definition, TICK + 1, false)?;
        if source_change {
            sample.operations.generation += 1;
        } else {
            let mut observed = LiveOperationsObservation::decode_profile(
                &sample.capture,
                sample.operations.generation,
                sample.operations.df_version.clone(),
                sample.operations.dfhack_version.clone(),
                OperationsProfile::PagedV1_4,
            )?;
            observed.buildings.pop();
            observed.items.pop();
            observed.jobs.jobs.pop();
            sample.capture = observed.encode_profile(OperationsProfile::PagedV1_4)?;
        }
        h.io.samples.push_back(sample);
        let rejected = h.prepare()?;
        assert_eq!(rejected["result"]["ok"], false, "{rejected}");
        assert!(!h.io.game.borrow().calls.contains(&"prepare"));
        assert!(!h.io.game.borrow().calls.contains(&"commit"));
        let monitor = h.state.completion.take().ok_or_else(missing_fixture)?;
        assert_eq!(monitor.store.progress().phase, "invalidated");
        assert_eq!(monitor.store.progress().observations, 2);
        let reason = monitor.store.progress().reason;
        drop(monitor);
        let c = h.state.context(false, None)?;
        let reopened = open_private_monitor(
            h.config
                .completion_path
                .as_ref()
                .ok_or_else(missing_fixture)?,
            &c,
            BuildMode::Recover,
            None,
        )?;
        assert_eq!(reopened.progress().phase, "invalidated");
        assert_eq!(reopened.progress().reason, reason);
        assert_eq!(reopened.progress().observations, 2);
    }
    Ok(())
}

#[test]
fn configured_missing_or_replaced_clock_custody_never_permits_a_fresh_goal_sample() -> Result<()> {
    let mut h = setup_clock(3)?;
    let c = h.state.context(false, None)?;
    clock::ensure_for_sample(&mut h.state, &c)?;
    let path = h
        .state
        .clock_config
        .as_ref()
        .ok_or_else(missing_fixture)?
        .path
        .clone();
    let bytes = fs::read(&path).map_err(storage_error)?;
    fs::rename(&path, h.directory.file("clock-original")).map_err(storage_error)?;
    write_private(&path, &bytes)?;
    assert!(clock::ensure_for_sample(&mut h.state, &c).is_err());
    assert!(clock::blocks_sampling(&h.state));
    assert_eq!(
        h.state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .progress()
            .observations,
        1
    );
    drop(h.state.clock.take());
    assert!(clock::ensure_for_sample(&mut h.state, &c).is_err());
    assert!(clock::blocks_sampling(&h.state));
    assert!(h.io.game.borrow().calls.is_empty());
    Ok(())
}

#[test]
fn maximum_original_furniture_set_keeps_every_member_in_clock_review() -> Result<()> {
    let mut h = setup_clock(32)?;
    let result = h.prepare()?;
    assert_eq!(result["result"]["ok"], true, "{result}");
    assert_eq!(result["result"]["batch"]["total"], 32);
    assert_eq!(
        result["result"]["batch"]["steps"].as_array().map(Vec::len),
        Some(32)
    );
    assert_eq!(
        result["result"]["completion"]["assessments"]
            .as_array()
            .map(Vec::len),
        Some(32)
    );
    assert_eq!(
        result["result"]["construction_clock"]["readiness"]["pending_count"],
        32
    );
    assert_eq!(
        h.state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .spec()
            .origin()
            .receipts()
            .len(),
        32
    );
    assert_eq!(h.io.acquisitions, 1);
    assert!(!h.io.game.borrow().calls.contains(&"commit"));
    Ok(())
}

#[test]
fn revoked_or_expired_native_query_does_not_disclose_cached_original_or_clock_evidence()
-> Result<()> {
    for expired in [false, true] {
        let mut h = setup_clock(3)?;
        assert_eq!(h.prepare()?["result"]["ok"], true);
        let recovery = h
            .state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .recovery();
        if expired {
            for grant in &mut h.state.clock_grants {
                if grant.capability == Capability::Query {
                    grant.expires_at_tick = Some(GameTick(TICK));
                }
            }
        } else {
            h.state
                .clock_grants
                .retain(|grant| grant.capability != Capability::Query);
        }
        let original_calls = h.io.game.borrow().calls.clone();
        let denied = h.call(clock::Action::Inspect {
            id: h.id()?,
            head: recovery.head,
            offset: 0,
            limit: 1,
        })?;
        assert_eq!(denied["result"]["ok"], false, "{denied}");
        assert!(
            denied["result"].get("construction_clock").is_none(),
            "{denied}"
        );
        assert!(denied["result"].get("batch").is_none(), "{denied}");
        assert!(denied["result"].get("completion").is_none(), "{denied}");
        assert_eq!(h.io.game.borrow().calls, original_calls);
    }
    Ok(())
}

#[test]
fn unacknowledged_owned_result_withdraws_monitor_stability_and_clock_review_before_next_request()
-> Result<()> {
    let mut h = setup_clock(3)?;
    let original_id = h.id()?;
    let c = h.state.context(false, None)?;
    clock::ensure_for_sample(&mut h.state, &c)?;
    let monitor = h.state.completion.as_mut().ok_or_else(missing_fixture)?;
    let sample = linked_sample(monitor.store.definition(), TICK + 2, true)?;
    let mut admit = |_: &MonitorDefinition, _: &Progress| Ok(());
    let mut guard = |_: &Origin, _: &OperationContext| Ok(());
    monitor.store.begin_read(&c, &mut admit, &mut guard)?;
    monitor
        .store
        .publish_sample(&c, &sample, &mut admit, &mut guard)?;
    assert_eq!(monitor.store.progress().streak, 1);
    h.io.complete = true;
    h.io.game.borrow_mut().tick = TICK + 2;
    let prepared = h.prepare()?;
    assert_eq!(prepared["result"]["ok"], true, "{prepared}");
    assert_eq!(
        h.state
            .completion
            .as_ref()
            .ok_or_else(missing_fixture)?
            .store
            .progress()
            .streak,
        1
    );
    assert!(
        h.state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .prepared_seal()
            .is_some()
    );
    h.state.worker_acknowledgement = Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        false,
    )));
    let _ = h.state.context(false, None)?;
    assert!(h.state.worker_acknowledgement.is_none());
    assert!(
        h.state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .prepared_seal()
            .is_none()
    );
    assert!(
        h.state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .cached_view()
            .pending
    );
    let monitor = h.state.completion.as_ref().ok_or_else(missing_fixture)?;
    assert_eq!(monitor.store.progress().streak, 0);
    assert_eq!(monitor.store.progress().observations, 3);
    assert_eq!(monitor.store.definition().id(), original_id);
    assert_eq!(
        monitor.store.definition().goal().timing().deadline,
        TICK + 1000
    );
    assert!(!h.io.game.borrow().calls.contains(&"commit"));
    Ok(())
}

#[test]
fn final_worker_handoff_revalidates_exact_clock_head_and_named_file_after_callbacks() -> Result<()>
{
    for replace_file in [false, true] {
        let mut h = setup_clock(3)?;
        let c = h.state.context(false, None)?;
        let original = h
            .state
            .clock
            .as_ref()
            .ok_or_else(missing_fixture)?
            .recovery();
        verify_clock_handoff(&mut h.state, &c, Instant::now(), Some(original))?;
        let expected = if replace_file {
            let path = h
                .state
                .clock_config
                .as_ref()
                .ok_or_else(missing_fixture)?
                .path
                .clone();
            let bytes = fs::read(&path).map_err(storage_error)?;
            fs::rename(&path, h.directory.file("clock-before-callback")).map_err(storage_error)?;
            write_private(&path, &bytes)?;
            original
        } else {
            Recovery {
                journal_id: original.journal_id,
                head: Digest32::of_bytes(b"rendered packet had a different exact clock head"),
            }
        };
        assert!(verify_clock_handoff(&mut h.state, &c, Instant::now(), Some(expected)).is_err());
        assert_eq!(
            h.state
                .clock
                .as_ref()
                .ok_or_else(missing_fixture)?
                .recovery(),
            original
        );
        assert!(h.io.game.borrow().calls.is_empty());
    }
    Ok(())
}
