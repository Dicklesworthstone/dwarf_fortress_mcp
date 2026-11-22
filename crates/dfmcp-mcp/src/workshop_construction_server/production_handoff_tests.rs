//! Actual Runtime export over private Workshop/ACK/Run files. Native vectors
//! seed the public journal coordinators; no game or production readiness is
//! inferred from these tests. Each Runtime runs in its own environment process.
#![cfg(unix)]

use super::*;
use dfmcp_adapter::bounded_run::{
    RunObservation, RunPlan, RunRecord,
    journal::{RunBinding, RunMode},
    private_file::open_private_run_journal_exact,
    rpc::{RunManifest, RunSource},
};
use dfmcp_adapter::workshop_construction::{
    WorkshopNativeSummary, WorkshopPreparation, WorkshopProgress, WorkshopRecord,
    clock::{Clock, RunRequest, Target},
    journal::WorkshopDispatch,
    rpc::WorkshopManifest,
};
use dfmcp_core::FortressId;
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const CHILD: &str = "WORKSHOP_HANDOFF_TEST_CHILD";
const CASE: &str = "WORKSHOP_HANDOFF_TEST_CASE";
const SETTLED_TICK: u64 = 15000;

fn fixture_error() -> DfmcpError {
    error(ErrorCode::InternalInvariantViolation, "handoff fixture failure")
}

fn vector(name: &str) -> Result<Vec<u8>> {
    let values: Value = serde_json::from_str(include_str!(
        "../../../../tests/native/workshop/vectors.json"
    ))
    .map_err(|_| fixture_error())?;
    let raw = values[name].as_str().ok_or_else(fixture_error)?;
    if !raw.len().is_multiple_of(2) {
        return Err(fixture_error());
    }
    raw.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            u8::from_str_radix(
                std::str::from_utf8(pair).map_err(|_| fixture_error())?,
                16,
            )
            .map_err(|_| fixture_error())
        })
        .collect()
}

fn capture() -> Result<WorkshopCapture> {
    WorkshopCapture::decode(&vector("capture")?)
}

fn record(name: &str) -> Result<WorkshopRecord> {
    WorkshopRecord::decode(&vector(name)?)
}

fn progress(name: &str) -> Result<WorkshopProgress> {
    WorkshopProgress::decode(&vector(name)?)
}

fn fixture_context() -> Result<OperationContext> {
    let before = capture()?;
    Ok(OperationContext {
        session_id: SessionId::new(901),
        request_id: RequestId::new(1),
        anchor: anchor(&before),
        budget: WorkBudget {
            max_wall_millis: 60000,
            max_bytes: 512 * 1024 * 1024,
            max_entities: 8192,
            max_output_tokens: 32768,
            max_actions: 1,
            max_game_ticks: 1200,
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::Construct,
            Capability::ControlClock,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(before.fortress_id()),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Original {
    Empty,
    Prepared,
    Placed,
    Incomplete,
    Complete,
    UnacknowledgedComplete,
    Indeterminate,
    Discontinuous,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClockSeed {
    Absent,
    Empty,
    Prepared,
    Running,
    Settled,
    Unknown,
}

struct WorkshopNative {
    manifest: WorkshopManifest,
    outcome: &'static str,
    current: Option<WorkshopRecord>,
    sample: Option<WorkshopProgress>,
}

impl WorkshopSource for WorkshopNative {
    fn manifest(&self) -> &WorkshopManifest {
        &self.manifest
    }
    fn native_summary(&self) -> WorkshopNativeSummary {
        WorkshopNativeSummary {
            unresolved: false,
            retained_records: u16::from(self.current.is_some()),
        }
    }
    fn fence(&mut self) {}
    fn observe(
        &mut self,
        _: WorkshopSelection,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopCapture> {
        capture()
    }
    fn prepare(
        &mut self,
        _: &WorkshopPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopPreparation> {
        let prepared = record("prepared")?;
        self.current = Some(prepared.clone());
        WorkshopPreparation::new(prepared, false)
    }
    fn commit(
        &mut self,
        dispatch: WorkshopDispatch<'_>,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopRecord> {
        assert_ne!(dispatch.journal_head(), Digest32::ZERO);
        let placed = record(self.outcome)?;
        self.current = Some(placed.clone());
        Ok(placed)
    }
    fn query(
        &mut self,
        _: &WorkshopPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<Option<WorkshopRecord>> {
        Ok(self.current.clone())
    }
    fn cancel(
        &mut self,
        _: &WorkshopPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopRecord> {
        Err(fixture_error())
    }
    fn progress(
        &mut self,
        building: u32,
        item: u32,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopProgress> {
        assert_eq!((building, item), (70, 42));
        self.sample.take().ok_or_else(fixture_error)
    }
}

struct FixtureGuard(WorkshopAcknowledgements<PrivateBuildFile>);
impl WorkshopGuard for FixtureGuard {
    fn acknowledge(&mut self, pin: WorkshopPin, c: &OperationContext) -> Result<()> {
        self.0.acknowledge(pin, c)
    }
    fn check(
        &mut self,
        _: WorkshopStage,
        _: &WorkshopBinding,
        _: WorkshopSelection,
        _: Option<[u32; 3]>,
        c: &OperationContext,
    ) -> Result<()> {
        self.0.verify(c)
    }
}

fn run_observation(tick: u64, sequence: u64, paused: bool) -> Result<RunObservation> {
    let mut raw = b"DFMRO013".to_vec();
    for value in [41u64, sequence, tick] {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    raw.extend_from_slice(&[1, 1, u8::from(paused)]);
    RunObservation::decode(&raw)
}

fn run_record(plan: &RunPlan, phase: u8, reason: u8, tick: u64) -> Result<RunRecord> {
    let mut raw = b"DFMRE013".to_vec();
    raw.extend_from_slice(&(plan.key().len() as u16).to_be_bytes());
    raw.extend_from_slice(plan.key().as_bytes());
    raw.extend_from_slice(&plan.spec().game_ticks().to_be_bytes());
    raw.extend_from_slice(&plan.spec().wall_ms().to_be_bytes());
    raw.extend_from_slice(plan.before().canonical_bytes());
    raw.extend_from_slice(plan.digest().as_bytes());
    raw.extend_from_slice(plan.token());
    let known = matches!(phase, 1..=3);
    raw.extend_from_slice(&[
        phase,
        reason,
        u8::from(matches!(phase, 1..=3 | 5)),
        u8::from(phase == 3),
        u8::from(known),
    ]);
    raw.extend_from_slice(&(if known { tick } else { 0 }).to_be_bytes());
    let mut proof = b"dfmcp-bounded-run-receipt/1\0".to_vec();
    proof.extend_from_slice(&raw);
    raw.extend_from_slice(Digest32::of_bytes(&proof).as_bytes());
    RunRecord::decode(&raw)
}

struct RunNative {
    manifest: RunManifest,
    tick: u64,
    sequence: u64,
    paused: bool,
    current: Option<RunRecord>,
}
impl RunSource for RunNative {
    fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(SocketAddr::from(([127, 0, 0, 1], 5000)))
    }
    fn observe(&mut self, _: &OperationContext) -> Result<RunObservation> {
        run_observation(self.tick, self.sequence, self.paused)
    }
    fn prepare(&mut self, plan: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        let prepared = run_record(plan, 0, 0, 0)?;
        self.current = Some(prepared.clone());
        Ok(prepared)
    }
    fn commit(&mut self, plan: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        self.sequence += 1;
        self.paused = false;
        let running = run_record(plan, 1, 0, self.tick)?;
        self.current = Some(running.clone());
        Ok(running)
    }
    fn query(&mut self, _: &RunPlan, _: &OperationContext) -> Result<Option<RunRecord>> {
        Ok(self.current.clone())
    }
    fn cancel(&mut self, _: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        Err(fixture_error())
    }
}

pub(crate) struct Fixture {
    directory: PathBuf,
    config: Value,
}
impl Fixture {
    pub(crate) fn configuration(&self) -> Value {
        self.config.clone()
    }
}

/// Reuse the actual independently acknowledged original for staged-owner tests.
pub(crate) fn completed_original_fixture() -> Result<Fixture> {
    fixture(Original::Complete, ClockSeed::Absent)
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn seed_clock(
    directory: &std::path::Path,
    binding: &WorkshopBinding,
    pin: WorkshopPin,
    c: &OperationContext,
    seed: ClockSeed,
) -> Result<Option<Value>> {
    if seed == ClockSeed::Absent {
        return Ok(None);
    }
    let manifest = RunManifest {
        generation: 41,
        df_version: "test-df".to_owned(),
        dfhack_version: "test-dfhack".to_owned(),
    };
    let mut native = c.clone();
    native.anchor.fortress_id = FortressId::NIL;
    native.grants = [Capability::Query, Capability::Plan, Capability::ControlClock]
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
        .collect();
    let journal = open_private_run_journal_exact(
        &directory.join("workshop.clock.journal"),
        &native,
        RunMode::Control,
        Some(RunBinding::new(binding.endpoint(), manifest.clone())?),
        true,
    )?;
    let target = Target::new(pin.journal_id, binding.clone(), record("placed")?, 19000)?;
    let mut owner = Clock::new(journal, target, None, c, &native)?;
    if seed != ClockSeed::Empty {
        let mut source = RunNative {
            manifest,
            tick: capture()?.tick(),
            sequence: 3,
            paused: true,
            current: None,
        };
        let request = RunRequest {
            target_id: owner.target().id(),
            game_ticks: 10,
            wall_millis: 500,
        };
        let prepared = owner.prepare_guarded(&mut source, &request, c, &native, |_, _, _| {
            progress("progress_stage0")
        })?;
        let seal = prepared
            .active()
            .and_then(|run| run.review_seal)
            .ok_or_else(fixture_error)?;
        if seed != ClockSeed::Prepared {
            owner.commit_guarded(&mut source, seal, true, c, &native, |_, _, _| {
                progress("progress_stage0")
            })?;
            if matches!(seed, ClockSeed::Settled | ClockSeed::Unknown) {
                let plan = source.current.as_ref().ok_or_else(fixture_error)?.plan().clone();
                source.current = Some(if seed == ClockSeed::Settled {
                    source.tick = SETTLED_TICK;
                    source.paused = true;
                    run_record(&plan, 3, 1, SETTLED_TICK)?
                } else {
                    source.manifest.generation += 1;
                    run_record(&plan, 5, 7, 0)?
                });
                owner.reconcile(&mut source, c, &native)?;
            }
        }
    }
    let recovery = owner.recovery();
    Ok(Some(json!({
        "initialize":false,"generation":41,"deadline_tick":19000,
        "recovery":{"journal_id":recovery.journal_id.to_hex(),"head":recovery.head.to_hex()}
    })))
}

fn fixture(original: Original, clock_seed: ClockSeed) -> Result<Fixture> {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let directory = std::env::temp_dir().join(format!(
        "dfmcp-workshop-handoff-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(|_| fixture_error())?;
    let mut fixture = Fixture {
        directory,
        config: Value::Null,
    };
    let c = fixture_context()?;
    let before = capture()?;
    let binding = WorkshopBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        "test-df",
        "test-dfhack",
        &before,
    )?;
    let ack = open_private_acknowledgements(
        &fixture.directory.join("workshop.ack"),
        &c,
        WorkshopMode::Control,
        true,
        None,
    )?;
    let mut guard = FixtureGuard(ack);
    let mut journal = open_private_workshop(
        &fixture.directory.join("workshop.journal"),
        &c,
        WorkshopMode::Control,
        Some(binding.clone()),
        [23; 32],
        None,
    )?;
    guard.acknowledge(journal.pin(), &c)?;
    if original != Original::Empty {
        let plan = WorkshopPlan::new("workshop-001", before.clone())?;
        let mut source = WorkshopNative {
            manifest: WorkshopManifest {
                endpoint: binding.endpoint(),
                fortress: before.fortress().clone(),
                generation: before.generation(),
                df_version: "test-df".to_owned(),
                dfhack_version: "test-dfhack".to_owned(),
            },
            outcome: if original == Original::Indeterminate {
                "indeterminate"
            } else {
                "placed"
            },
            current: None,
            sample: None,
        };
        journal.prepare(&mut source, &plan, &c, &mut guard)?;
        guard.acknowledge(journal.pin(), &c)?;
        if original != Original::Prepared {
            journal.commit(&mut source, plan.key(), plan.digest(), &c, &mut guard)?;
            guard.acknowledge(journal.pin(), &c)?;
            if matches!(
                original,
                Original::Incomplete | Original::Complete | Original::UnacknowledgedComplete
                    | Original::Discontinuous
            ) {
                let mut sample = vector(if original == Original::Incomplete {
                    "progress_stage0"
                } else {
                    "progress_complete"
                })?;
                if original == Original::Discontinuous {
                    source.manifest.generation += 1;
                    sample[8..16].copy_from_slice(&source.manifest.generation.to_be_bytes());
                }
                source.sample = Some(WorkshopProgress::decode(&sample)?);
                journal.sample(&mut source, plan.key(), plan.digest(), &c, &mut guard)?;
                if original != Original::UnacknowledgedComplete {
                    guard.acknowledge(journal.pin(), &c)?;
                }
            }
        }
    }
    let clock_config = seed_clock(&fixture.directory, &binding, journal.pin(), &c, clock_seed)?;
    let acknowledged = guard.0.latest().ok_or_else(fixture_error)?;
    let dimensions = before.dimensions();
    fixture.config = json!({
        "schema":"dfmcp.workshop-mcp-development/1",
        "directory":fixture.directory,
        "endpoint":binding.endpoint().to_string(),
        "initialize":false,"acknowledged":pin_json(acknowledged),
        "world_folder":before.folder(),"site_id":before.site(),
        "df_version":"test-df","dfhack_version":"test-dfhack",
        "generation":before.generation(),"map_dimensions":dimensions,
        "item_id":before.selection().item_id(),"center":before.selection().center(),
        "scope":[0,0,0,dimensions[0]-1,dimensions[1]-1,dimensions[2]-1],
        "protected":[],"authority_expires_tick":20000,"lease_ticks":1200,
        "checkpoint":"disposable-fortress-no-checkpoint","clock":clock_config,
    });
    Ok(fixture)
}

fn isolated(
    test: &str,
    case: &str,
    original: Original,
    clock_seed: ClockSeed,
    mode: WorkshopMode,
    check: impl FnOnce(&mut Runtime) -> Result<()>,
) -> Result<()> {
    if let Ok(selected) = std::env::var(CHILD) {
        if selected != test || std::env::var(CASE).ok().as_deref() != Some(case) {
            return Ok(());
        }
        return crate::run_with_runtime_cx(|_| async move {
            assert!(std::env::var_os(TOKEN).is_none());
            assert!(std::env::var_os(clock::TOKEN).is_none());
            assert!(std::env::var_os(BUILD).is_none());
            assert!(std::env::var_os(clock::ALLOW).is_none());
            let config = Config::environment()?;
            let mut runtime = Runtime::open(config, mode, SessionId::new(902), fixture_context()?.budget)?;
            check(&mut runtime)
        })
        .map_err(|_| fixture_error())?;
    }
    let fixture = fixture(original, clock_seed)?;
    let mut command = Command::new(std::env::current_exe().map_err(|_| fixture_error())?);
    command
        .arg("--exact")
        .arg(format!(
            "workshop_construction_server::runtime::production_handoff_tests::{test}"
        ))
        .arg("--nocapture");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("DFMCP_") {
            command.env_remove(key);
        }
    }
    command
        .env(CHILD, test)
        .env(CASE, case)
        .env(OPT_IN, "1")
        .env(CONFIG, fixture.config.to_string());
    if clock_seed != ClockSeed::Absent {
        command.env(clock::OPT_IN, "1");
    }
    let output = command.output().map_err(|_| fixture_error())?;
    assert!(
        output.status.success(),
        "{case}: stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "the isolated actual-source test was not selected"
    );
    Ok(())
}

fn custody(runtime: &Runtime) -> Result<Vec<Vec<u8>>> {
    let mut files = vec!["workshop.journal", "workshop.ack"];
    if runtime.clock.is_some() {
        files.push("workshop.clock.journal");
    }
    files.into_iter()
        .map(|name| fs::read(runtime.config.input.directory.join(name)).map_err(|_| fixture_error()))
        .collect()
}

#[test]
fn handoff_exports_exact_original_without_native_credentials_or_custody_writes() -> Result<()> {
    for mode in [WorkshopMode::Control, WorkshopMode::Recover, WorkshopMode::Offline] {
        isolated(
            "handoff_exports_exact_original_without_native_credentials_or_custody_writes",
            mode.as_str(), Original::Complete, ClockSeed::Absent, mode,
            |runtime| {
                let before = custody(runtime)?;
                let c = runtime.context()?;
                let exported = runtime.production_handoff(&c)?;
                assert_eq!(exported.directory(), runtime.config.input.directory.as_path());
                assert_eq!(exported.binding(), runtime.journal.binding());
                assert_eq!(exported.pin(), runtime.journal.pin());
                assert_eq!(exported.original_key(), "workshop-001");
                assert_eq!(exported.original_plan_digest(), record("placed")?.plan().digest());
                assert_ne!(exported.digest(), Digest32::ZERO);
                let metadata = exported.to_json();
                assert_eq!(metadata["original_receipt"], record("placed")?.receipt().to_hex());
                assert_eq!(metadata["building_id"], 70);
                assert_eq!(metadata["material_item_id"], 42);
                assert_eq!(metadata["historical_completion"]["witness"], progress("progress_complete")?.witness().to_hex());
                assert_eq!(metadata["historical_completion_proven"], true);
                for key in ["current_completion_proven", "current_pause_proven", "production_ready", "mutation_authority"] {
                    assert_eq!(metadata[key], false, "{key}");
                }
                assert_eq!(metadata["historical_completion"]["fresh_for_current_call"], false);
                assert_eq!(metadata["native_calls"], 0);
                assert_eq!(metadata["workshop_prerequisite"]["acknowledged"], pin_json(exported.pin()));
                let view = runtime.production_handoff_view(&c)?;
                assert_eq!(view["production_handoff"], metadata);
                assert_eq!(view["current_goal_available"], false);
                assert_eq!(view["original"]["fresh_completion_proven"], false);
                assert_eq!(view["original"]["native_record_hex"], hex(record("placed")?.canonical_bytes()));
                assert_eq!(view["original"]["source"]["canonical_capture_hex"], hex(capture()?.canonical_bytes()));
                assert_eq!(view["original"]["sample_hex"], hex(progress("progress_complete")?.canonical_bytes()));
                assert!(runtime.source.is_none());
                assert!(!runtime.journal.has_permit("workshop-001"));
                assert_eq!(custody(runtime)?, before);
                let mut later = c;
                later.anchor.tick = GameTick(exported.authority_tick_floor() + 7);
                let later = runtime.production_handoff(&later)?;
                assert_eq!(later.digest(), exported.digest());
                assert_eq!(later.authority_tick_floor(), exported.authority_tick_floor() + 7);
                assert_eq!(custody(runtime)?, before);
                Ok(())
            },
        )?;
    }
    Ok(())
}

#[test]
fn handoff_refuses_unfinished_uncertain_or_discontinuous_originals() -> Result<()> {
    for (name, original) in [
        ("empty", Original::Empty), ("prepared", Original::Prepared),
        ("placed", Original::Placed), ("incomplete", Original::Incomplete),
        ("indeterminate", Original::Indeterminate), ("discontinuous", Original::Discontinuous),
    ] {
        isolated(
            "handoff_refuses_unfinished_uncertain_or_discontinuous_originals",
            name, original, ClockSeed::Absent, WorkshopMode::Offline,
            |runtime| {
                let before = custody(runtime)?;
                let c = runtime.context()?;
                assert!(runtime.production_handoff(&c).is_err());
                assert!(runtime.production_handoff_view(&c).is_err());
                assert_eq!(custody(runtime)?, before);
                assert!(runtime.source.is_none());
                assert!(!runtime.journal.has_permit("workshop-001"));
                Ok(())
            },
        )?;
    }
    Ok(())
}

#[test]
fn handoff_requires_current_query_owner_scope_expiry_and_complete_work_budget() -> Result<()> {
    isolated(
        "handoff_requires_current_query_owner_scope_expiry_and_complete_work_budget",
        "authority", Original::Complete, ClockSeed::Absent, WorkshopMode::Offline,
        |runtime| {
            let before = custody(runtime)?;
            let mut query = runtime.context()?;
            query.grants.retain(|grant| grant.capability == Capability::Query);
            runtime.production_handoff(&query)?;
            for changed in 0..6 {
                let mut invalid = query.clone();
                match changed {
                    0 => invalid.session_id = SessionId::new(903),
                    1 => invalid.grants.clear(),
                    2 => invalid.anchor.fortress_id = FortressId::NIL,
                    3 => invalid.grants[0].scope.fortress_id = Some(FortressId::NIL),
                    4 => {
                        invalid.anchor.tick = GameTick(0);
                        invalid.grants[0].expires_at_tick = Some(GameTick(capture()?.tick()));
                    }
                    _ => invalid.cancellation_requested = true,
                }
                assert!(runtime.production_handoff(&invalid).is_err(), "{changed}");
                assert_eq!(custody(runtime)?, before);
            }
            let mut small = query.clone();
            small.budget.max_bytes = 1;
            assert_eq!(runtime.production_handoff(&small).err().map(|e| e.code), Some(ErrorCode::BudgetExceeded));
            runtime.production_handoff(&query)?;
            assert_eq!(custody(runtime)?, before);
            Ok(())
        },
    )
}

#[test]
fn handoff_requires_the_exact_ack_and_never_repairs_changed_custody() -> Result<()> {
    isolated(
        "handoff_requires_the_exact_ack_and_never_repairs_changed_custody",
        "unacknowledged", Original::UnacknowledgedComplete, ClockSeed::Absent, WorkshopMode::Offline,
        |runtime| {
            let before = custody(runtime)?;
            let c = runtime.context()?;
            assert_eq!(runtime.production_handoff(&c).err().map(|e| e.code), Some(ErrorCode::CorruptLedger));
            assert_eq!(custody(runtime)?, before);
            Ok(())
        },
    )?;
    isolated(
        "handoff_requires_the_exact_ack_and_never_repairs_changed_custody",
        "changed", Original::Complete, ClockSeed::Absent, WorkshopMode::Offline,
        |runtime| {
            let before = custody(runtime)?;
            let path = runtime.config.input.directory.join("workshop.ack");
            let mut changed = before[1].clone();
            changed[0] ^= 1;
            fs::write(&path, &changed).map_err(|_| fixture_error())?;
            let c = runtime.context()?;
            assert_eq!(runtime.production_handoff(&c).err().map(|e| e.code), Some(ErrorCode::CorruptLedger));
            assert_eq!(custody(runtime)?[1], changed);
            fs::write(&path, &before[1]).map_err(|_| fixture_error())?;
            assert!(runtime.production_handoff(&c).is_err());
            assert_eq!(custody(runtime)?, before);
            Ok(())
        },
    )
}

#[test]
fn handoff_retains_settled_clock_identity_and_recovered_authority_floor() -> Result<()> {
    for (name, seed) in [("empty", ClockSeed::Empty), ("settled", ClockSeed::Settled)] {
        isolated(
            "handoff_retains_settled_clock_identity_and_recovered_authority_floor",
            name, Original::Complete, seed, WorkshopMode::Offline,
            move |runtime| {
                let before = custody(runtime)?;
                let reference = runtime.clock.as_ref().ok_or_else(fixture_error)?.reference();
                let c = runtime.context()?;
                if seed == ClockSeed::Settled {
                    assert!(c.anchor.tick.get() >= SETTLED_TICK);
                    let mut expired = c.clone();
                    expired.anchor.tick = GameTick(0);
                    for grant in &mut expired.grants {
                        grant.expires_at_tick = Some(GameTick(SETTLED_TICK - 1));
                    }
                    assert!(runtime.production_handoff(&expired).is_err());
                    assert!(runtime.context()?.anchor.tick.get() >= SETTLED_TICK);
                }
                let handoff = runtime.production_handoff(&c)?;
                let exported = handoff.to_json();
                assert_eq!(exported["clock"]["recovery"], reference["recovery"]);
                assert_eq!(exported["clock"]["generation"], 41);
                assert_eq!(exported["clock"]["verified"], true);
                assert_eq!(exported["clock"]["pending"], false);
                assert_eq!(exported["clock"]["unresolved"], false);
                assert_eq!(exported["clock"]["current_pause_proven"], false);
                if seed == ClockSeed::Settled {
                    assert!(handoff.authority_tick_floor() >= SETTLED_TICK);
                    assert_eq!(exported["historical_completion"]["tick"], progress("progress_complete")?.tick());
                    assert!(progress("progress_complete")?.tick() < SETTLED_TICK);
                }
                assert_eq!(custody(runtime)?, before);
                let view = runtime.production_handoff_view(&c)?;
                assert_eq!(view["production_handoff"]["clock"]["recovery"], reference["recovery"]);
                assert_eq!(view["current_goal_available"], false);
                assert_eq!(custody(runtime)?, before);
                Ok(())
            },
        )?;
    }
    Ok(())
}

#[test]
fn handoff_refuses_pending_unknown_and_fenced_clocks_without_changing_files() -> Result<()> {
    for (name, seed) in [
        ("prepared", ClockSeed::Prepared), ("running", ClockSeed::Running),
        ("unknown", ClockSeed::Unknown), ("fenced", ClockSeed::Empty),
    ] {
        isolated(
            "handoff_refuses_pending_unknown_and_fenced_clocks_without_changing_files",
            name, Original::Complete, seed, WorkshopMode::Offline,
            move |runtime| {
                if name == "fenced" {
                    runtime.clock.as_mut().ok_or_else(fixture_error)?.fence_advancement();
                }
                let before = custody(runtime)?;
                let c = runtime.context()?;
                assert_eq!(runtime.production_handoff(&c).err().map(|e| e.code), Some(ErrorCode::EffectIndeterminate));
                assert_eq!(custody(runtime)?, before);
                let retained = runtime.failure_reference();
                assert_eq!(retained["clock"]["verified"], false);
                assert_eq!(retained["clock"]["current_pause_proven"], false);
                assert_eq!(retained["clock"]["automatic_redispatch"], false);
                assert!(retained["clock"]["recovery"]["journal_id"].is_string());
                assert!(retained["clock"]["recovery"]["head"].is_string());
                Ok(())
            },
        )?;
    }
    Ok(())
}
