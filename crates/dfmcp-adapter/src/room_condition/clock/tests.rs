//! Real native intent/dispatch custody for separately identified room clocks.
use super::super::tests::{change_map, evidence, fixture};
use super::*;
use crate::bounded_run::journal::RunState;
use crate::bounded_run::rpc::RunManifest;
use crate::bounded_run::{RunObservation, RunPhase, RunRecord};
use crate::construction_plan::{self as construction, Timing, fixtures};
use crate::live_jobs::LiveJob;
use dfmcp_core::{CapabilityGrant, CapabilityScope, FortressId, MapCoord};
use dfmcp_world::map_region::{Cell, Shape};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn error(code: ErrorCode, text: &str) -> DfmcpError {
    DfmcpError::new(code, text)
}
fn corrupt() -> DfmcpError {
    error(ErrorCode::CorruptLedger, "invalid room-clock fixture")
}
#[derive(Default)]
struct Bytes {
    cursor: Cursor<Vec<u8>>,
    writes: usize,
    flushes: usize,
    syncs: usize,
    fail_sync: bool,
}
#[derive(Clone, Default)]
struct Memory(Rc<RefCell<Bytes>>);
impl Memory {
    fn from_bytes(raw: Vec<u8>) -> Self {
        let value = Self::default();
        value.0.borrow_mut().cursor = Cursor::new(raw);
        value
    }
    fn snapshot_bytes(&self) -> Vec<u8> {
        self.0.borrow().cursor.get_ref().clone()
    }
    fn operations(&self) -> (usize, usize, usize) {
        let b = self.0.borrow();
        (b.writes, b.flushes, b.syncs)
    }
    fn damage(&self) {
        self.0.borrow_mut().cursor.get_mut()[0] ^= 1;
    }
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.borrow_mut().cursor.read(out)
    }
}
impl Seek for Memory {
    fn seek(&mut self, at: SeekFrom) -> io::Result<u64> {
        self.0.borrow_mut().cursor.seek(at)
    }
}
impl Write for Memory {
    fn write(&mut self, raw: &[u8]) -> io::Result<usize> {
        let mut bytes = self.0.borrow_mut();
        bytes.writes += 1;
        bytes.cursor.write(raw)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.borrow_mut().flushes += 1;
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let mut bytes = self.0.borrow_mut();
        bytes.syncs += 1;
        if std::mem::take(&mut bytes.fail_sync) {
            Err(io::Error::other("lost sync acknowledgment"))
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no repair"))
    }
}

fn observation(generation: u64, sequence: u64, tick: u64, paused: bool) -> Result<RunObservation> {
    let mut raw = b"DFMRO013".to_vec();
    for value in [generation, sequence, tick] {
        raw.extend_from_slice(&value.to_be_bytes());
    }
    raw.extend_from_slice(&[1, 1, u8::from(paused)]);
    RunObservation::decode(&raw)
}
fn record(plan: &RunPlan, phase: u8, reason: u8, tick: u64) -> Result<RunRecord> {
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

struct Source {
    storage: Memory,
    manifest: RunManifest,
    endpoint: SocketAddr,
    tick: u64,
    sequence: u64,
    paused: bool,
    native: Option<RunRecord>,
    observes: usize,
    prepares: usize,
    commits: usize,
    queries: usize,
    cancels: usize,
    lose_commit: bool,
    absent: bool,
    fail_receipt_sync: bool,
}
impl Source {
    fn new(storage: Memory) -> Self {
        Self {
            storage,
            manifest: RunManifest {
                generation: 41,
                df_version: "test-df".into(),
                dfhack_version: "test-dfhack".into(),
            },
            endpoint: SocketAddr::from(([127, 0, 0, 1], 5000)),
            tick: fixtures::TICK,
            sequence: 3,
            paused: true,
            native: None,
            observes: 0,
            prepares: 0,
            commits: 0,
            queries: 0,
            cancels: 0,
            lose_commit: false,
            absent: false,
            fail_receipt_sync: false,
        }
    }
    fn current(&self) -> Result<RunObservation> {
        observation(
            self.manifest.generation,
            self.sequence,
            self.tick,
            self.paused,
        )
    }
    fn assert_durable(&self, p: &RunPlan, state: RunState, c: &OperationContext) -> Result<()> {
        let mut reopened =
            RunJournal::open(self.storage.clone(), c, RunMode::Offline, None, false)?;
        let found = reopened
            .records(c)?
            .into_iter()
            .find(|run| run.plan() == p)
            .ok_or_else(corrupt)?;
        assert_eq!(found.state(), state);
        assert!(self.storage.operations().2 > 0);
        Ok(())
    }
    fn finish(&mut self, tick: u64) -> Result<()> {
        let p = self.native.as_ref().ok_or_else(corrupt)?.plan().clone();
        self.native = Some(record(&p, 3, 1, tick)?);
        self.tick = tick;
        self.paused = true;
        Ok(())
    }
}
impl RunSource for Source {
    fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(self.endpoint)
    }
    fn observe(&mut self, _: &OperationContext) -> Result<RunObservation> {
        self.observes += 1;
        self.current()
    }
    fn prepare(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.assert_durable(p, RunState::Intent, c)?;
        assert_eq!(&self.current()?, p.before());
        self.prepares += 1;
        let value = record(p, 0, 0, 0)?;
        self.native = Some(value.clone());
        Ok(value)
    }
    fn commit(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.assert_durable(p, RunState::DispatchStarted, c)?;
        assert_eq!(&self.current()?, p.before());
        self.commits += 1;
        self.sequence += 1;
        self.paused = false;
        let value = record(p, 1, 0, self.tick)?;
        self.native = Some(value.clone());
        if self.fail_receipt_sync {
            self.storage.0.borrow_mut().fail_sync = true;
        }
        if self.lose_commit {
            Err(error(
                ErrorCode::AdapterUnavailable,
                "lost native commit reply",
            ))
        } else {
            Ok(value)
        }
    }
    fn query(&mut self, p: &RunPlan, _: &OperationContext) -> Result<Option<RunRecord>> {
        self.queries += 1;
        if let Some(record) = &self.native {
            assert_eq!(record.plan(), p);
        }
        Ok(if self.absent {
            None
        } else {
            self.native.clone()
        })
    }
    fn cancel(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.assert_durable(p, RunState::CancelRequested, c)?;
        self.cancels += 1;
        self.tick += 1;
        self.paused = true;
        let already_prepared = self
            .native
            .as_ref()
            .is_some_and(|r| r.phase() == RunPhase::Prepared);
        let value = record(p, if already_prepared { 4 } else { 3 }, 3, self.tick)?;
        self.native = Some(value.clone());
        Ok(value)
    }
}

fn contexts(spec: &Definition) -> (OperationContext, OperationContext) {
    let mut c = fixtures::context(spec.goal());
    c.anchor.tick = GameTick(fixtures::TICK);
    c.budget.max_bytes = 512 * 1024 * 1024;
    c.budget.max_game_ticks = 1200;
    c.budget.max_actions = 64;
    for capability in [
        Capability::Observe,
        Capability::Plan,
        Capability::ControlClock,
    ] {
        c.grants.push(CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(c.anchor.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        });
    }
    let mut n = c.clone();
    n.anchor.fortress_id = FortressId::NIL;
    n.grants = [
        Capability::Query,
        Capability::Plan,
        Capability::ControlClock,
    ]
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
    (c, n)
}

fn source_identity(definition: &Definition) -> Result<SourceIdentity> {
    let sample = evidence(definition, fixtures::TICK)?;
    Ok(SourceIdentity {
        furniture: construction::SourceIdentity {
            furniture_generation: sample.furnishings.before.generation,
            operations: sample.furnishings.operations,
        },
        map: sample.before,
    })
}
fn open(
    storage: Memory,
    source: &Source,
    n: &OperationContext,
    mode: RunMode,
    create: bool,
) -> Result<RunJournal<Memory>> {
    RunJournal::open(
        storage,
        n,
        mode,
        Some(RunBinding::new(source.endpoint, source.manifest.clone())?),
        create,
    )
}
fn setup(
    count: usize,
) -> Result<(
    Clock<Memory>,
    Memory,
    Source,
    OperationContext,
    OperationContext,
)> {
    let (definition, _) = fixture(count)?;
    let (c, n) = contexts(&definition);
    let bytes = Memory::default();
    let source = Source::new(bytes.clone());
    let journal = open(bytes.clone(), &source, &n, RunMode::Control, true)?;
    let pin = source_identity(&definition)?;
    let clock = Clock::new(journal, definition, pin, None, &c, &n)?;
    Ok((clock, bytes, source, c, n))
}
fn request(definition: &Definition) -> RunRequest {
    RunRequest {
        monitor_id: definition.id(),
        game_ticks: 10,
        wall_millis: 500,
    }
}
fn retained(definition: &Definition, c: &OperationContext) -> Result<Progress> {
    super::super::advance(
        &Progress::new(definition).begin_read()?,
        definition,
        &evidence(definition, fixtures::TICK)?,
        c,
    )
}
fn ready(definition: &Definition, tick: u64, c: &OperationContext) -> Result<RoomClockSample> {
    Ok(RoomClockSample {
        progress: retained(definition, c)?,
        sample: evidence(definition, tick)?,
    })
}
fn prepare(
    clock: &mut Clock<Memory>,
    source: &mut Source,
    c: &OperationContext,
    n: &OperationContext,
) -> Result<Digest32> {
    let definition = clock.definition().clone();
    let view = clock.prepare_guarded(source, &request(&definition), c, n, |_, plan, current| {
        ready(
            &definition,
            plan.before().tick().ok_or_else(stale)?,
            current,
        )
    })?;
    view.active()
        .and_then(|run| run.review_seal)
        .ok_or_else(stale)
}
fn commit(
    clock: &mut Clock<Memory>,
    source: &mut Source,
    seal: Digest32,
    c: &OperationContext,
    n: &OperationContext,
) -> Result<ClockView> {
    let definition = clock.definition().clone();
    clock.commit_guarded(source, seal, true, c, n, |_, plan, current| {
        ready(
            &definition,
            plan.before().tick().ok_or_else(stale)?,
            current,
        )
    })
}
fn field_hash(domain: &[u8], fields: &[&[u8]]) -> Digest32 {
    let mut raw = domain.to_vec();
    raw.push(0);
    for field in fields {
        raw.extend_from_slice(&(field.len() as u64).to_be_bytes());
        raw.extend_from_slice(field);
    }
    Digest32::of_bytes(&raw)
}
fn with_timing(definition: &Definition, timing: Timing) -> Result<Definition> {
    Definition::new(construction::origin::MonitorDefinition::new(
        definition.origin().clone(),
        construction::Goal::new(definition.goal().records().to_vec(), timing)?,
    )?)
}
fn pending(definition: &Definition, tick: u64) -> Result<Sample> {
    let mut sample = evidence(definition, tick)?;
    let mut observed = fixtures::observation(definition.goal(), tick);
    let original = definition.goal().records()[0]
        .insertion()
        .ok_or_else(stale)?;
    observed.buildings[0].build_stage = original.max_stage() as i32 - 1;
    observed.jobs.jobs.push(LiveJob {
        native_id: original.job_id(),
        job_type: 1,
        type_key: "ConstructBuilding".into(),
        reaction: String::new(),
        suspended: false,
        repeating: false,
        position: MapCoord::new(15, 15, 2),
        worker_native_id: None,
        holder_native_id: Some(original.building_id()),
        completion_timer: -1,
        attached_item_count: 0,
        required_item_filter_count: 0,
    });
    sample.furnishings = fixtures::with_observation(definition.goal(), observed);
    Ok(sample)
}

#[test]
fn full_room_identity_and_legacy_furniture_bytes_remain_separate() -> Result<()> {
    for count in [1, 32] {
        let (mut clock, storage, mut native, c, n) = setup(count)?;
        let definition = clock.definition().clone();
        let pin = clock.source().clone();
        let seal = prepare(&mut clock, &mut native, &c, &n)?;
        let view = clock.inspect(&c, &n)?;
        assert_eq!(view.monitor_id, definition.id());
        let run = view.active().ok_or_else(stale)?;
        let target = field_hash(
            b"dfmcp-room-clock-target/1",
            &[
                definition.canonical_bytes(),
                &source_bytes(&definition, &pin)?,
                view.recovery.journal_id.as_bytes(),
            ],
        );
        assert_eq!(
            run.entry.plan().key(),
            format!(
                "room-clock-{}",
                field_hash(
                    b"dfmcp-room-clock-key/1",
                    &[target.as_bytes(), run.entry.plan().digest().as_bytes()]
                )
                .to_hex()
            )
        );
        let readiness = view.readiness.as_ref().ok_or_else(stale)?;
        assert_eq!(readiness.complete_count(), count as u32);
        assert!(
            readiness
                .terrain()
                .is_some_and(TerrainAssessment::condition_met)
        );
        assert_eq!(
            seal,
            field_hash(
                b"dfmcp-room-clock-review/1",
                &[
                    target.as_bytes(),
                    &run.entry.plan().canonical_bytes(),
                    readiness.digest().as_bytes(),
                    &c.session_id.get().to_be_bytes(),
                ]
            )
        );
        let journal = open(storage.clone(), &native, &n, RunMode::Offline, false)?;
        assert!(
            shared::Clock::new(
                journal,
                definition.original().clone(),
                pin.furniture.operations.clone(),
                Some(view.recovery),
                &c,
                &n
            )
            .is_err()
        );

        // The old target/key/review bytes still use the exact old domains and
        // field sequence even when the original batch carries a room intent.
        let legacy_bytes = Memory::default();
        let mut legacy_source = Source::new(legacy_bytes.clone());
        let journal = open(
            legacy_bytes.clone(),
            &legacy_source,
            &n,
            RunMode::Control,
            true,
        )?;
        let mut legacy = shared::Clock::new(
            journal,
            definition.original().clone(),
            pin.furniture.operations.clone(),
            None,
            &c,
            &n,
        )?;
        let old_request = RunRequest {
            monitor_id: definition.original().id(),
            ..request(&definition)
        };
        let old = legacy.prepare_guarded(&mut legacy_source, &old_request, &c, &n, |_, _, _| {
            Ok(shared::ClockSample {
                progress: retained(&definition, &c)?.construction,
                sample: evidence(&definition, fixtures::TICK)?.furnishings,
            })
        })?;
        let old_target = field_hash(
            b"dfmcp-furniture-clock-target/1",
            &[
                definition.original().canonical_bytes(),
                &pin.furniture.operations.canonical_bytes()?,
                old.recovery.journal_id.as_bytes(),
            ],
        );
        let old_run = old.active().ok_or_else(stale)?;
        let old_ready = old.readiness.as_ref().ok_or_else(stale)?;
        assert!(old_ready.terrain().is_none());
        assert_eq!(
            old_run.entry.plan().key(),
            format!(
                "furniture-clock-{}",
                field_hash(
                    b"dfmcp-furniture-clock-key/1",
                    &[
                        old_target.as_bytes(),
                        old_run.entry.plan().digest().as_bytes()
                    ]
                )
                .to_hex()
            )
        );
        assert_eq!(
            old_run.review_seal,
            Some(field_hash(
                b"dfmcp-furniture-clock-review/1",
                &[
                    old_target.as_bytes(),
                    &old_run.entry.plan().canonical_bytes(),
                    old_ready.digest().as_bytes(),
                    &c.session_id.get().to_be_bytes(),
                ]
            ))
        );
        let journal = open(legacy_bytes, &legacy_source, &n, RunMode::Offline, false)?;
        assert!(Clock::new(journal, definition, pin, Some(old.recovery), &c, &n).is_err());
        assert_eq!((native.commits, legacy_source.commits), (0, 0));
    }
    Ok(())
}

#[test]
fn whole_room_known_deficits_are_disclosed_while_unknown_terrain_refuses() -> Result<()> {
    let (definition, _) = fixture(32)?;
    let (c, _) = contexts(&definition);
    let pin = source_identity(&definition)?;
    let progress = retained(&definition, &c)?;
    for variant in 0..7 {
        let mut sample = evidence(&definition, fixtures::TICK + 5)?;
        let wall = definition.room().terrain().walls()[0];
        change_map(&mut sample, |map| {
            let index = map.map.region.index(wall).ok_or_else(stale)?;
            let Cell::Visible(mut tile) = map.map.cells[index] else {
                return Err(stale());
            };
            map.map.cells[index] = match variant {
                0 => {
                    tile.shape = Shape::Floor;
                    Cell::Visible(tile)
                }
                1 => {
                    tile.liquid_depth = 3;
                    Cell::Visible(tile)
                }
                2 => {
                    tile.dig_designation = 1;
                    Cell::Visible(tile)
                }
                3 => {
                    tile.building_occupancy = 1;
                    Cell::Visible(tile)
                }
                4 => {
                    tile.magma = true;
                    Cell::Visible(tile)
                }
                5 => Cell::Hidden,
                _ => Cell::Unallocated,
            };
            Ok(())
        })?;
        let result = capture_readiness(&definition, &pin, &progress, &sample, &c);
        if variant < 5 {
            let ready = result?;
            assert_eq!(ready.complete_count(), 32);
            assert_eq!(ready.pending_count(), 0);
            let terrain = ready.terrain().ok_or_else(stale)?;
            assert!(!terrain.condition_met());
            assert_eq!(terrain.deficit_count(), 1);
            assert_eq!(terrain.deficits[0].position, wall);
            assert!(ready.withdraws_stability());
        } else {
            assert!(result.is_err());
        }
        assert_eq!((progress.observations, progress.streak), (1, 1));
    }
    Ok(())
}

#[test]
fn both_room_guards_follow_durable_markers_and_commit_consumes_the_only_review() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup(32)?;
    let definition = clock.definition().clone();
    let mut edges = Vec::new();
    let prepared = clock.prepare_guarded(
        &mut source,
        &request(&definition),
        &c,
        &n,
        |edge, plan, current| {
            let mut opened = open(
                storage.clone(),
                &Source::new(storage.clone()),
                &n,
                RunMode::Offline,
                false,
            )?;
            assert_eq!(opened.records(&n)?[0].state(), RunState::Intent);
            edges.push(edge);
            ready(
                &definition,
                plan.before().tick().ok_or_else(stale)?,
                current,
            )
        },
    )?;
    let seal = prepared
        .active()
        .and_then(|run| run.review_seal)
        .ok_or_else(stale)?;
    let running =
        clock.commit_guarded(&mut source, seal, true, &c, &n, |edge, plan, current| {
            let mut opened = open(
                storage.clone(),
                &Source::new(storage.clone()),
                &n,
                RunMode::Offline,
                false,
            )?;
            assert_eq!(opened.records(&n)?[0].state(), RunState::DispatchStarted);
            edges.push(edge);
            ready(
                &definition,
                plan.before().tick().ok_or_else(stale)?,
                current,
            )
        })?;
    assert_eq!(edges, vec![Edge::Prepare, Edge::Commit]);
    assert_eq!(source.commits, 1);
    assert!(running.unresolved);
    assert!(!running.can_sample_goal());
    assert!(clock.prepared_seal().is_none());
    assert!(commit(&mut clock, &mut source, seal, &c, &n).is_err());
    assert_eq!(source.commits, 1);
    Ok(())
}

#[test]
fn whole_definition_and_first_independent_sources_cannot_change_on_reopen() -> Result<()> {
    let (mut clock, storage, mut native, c, n) = setup(1)?;
    prepare(&mut clock, &mut native, &c, &n)?;
    let definition = clock.definition().clone();
    let original_source = clock.source().clone();
    let pin = clock.recovery();
    for variant in 0..5 {
        let mut changed_source = original_source.clone();
        let mut changed_definition = definition.clone();
        match variant {
            0 => changed_source.map.generation += 1,
            1 => changed_source.furniture.operations.generation += 1,
            2 => changed_source.furniture.furniture_generation += 1,
            3 => changed_source.map.dfhack_version.push_str("-other"),
            _ => {
                let mut timing = definition.goal().timing().clone();
                timing.stable_span += 1;
                changed_definition = with_timing(&definition, timing)?;
            }
        }
        let journal = open(storage.clone(), &native, &n, RunMode::Offline, false)?;
        assert!(
            Clock::new(
                journal,
                changed_definition,
                changed_source,
                Some(pin),
                &c,
                &n
            )
            .is_err()
        );
    }
    let journal = open(storage, &native, &n, RunMode::Offline, false)?;
    let restored = Clock::new(journal, definition, original_source, Some(pin), &c, &n)?;
    assert!(restored.prepared_seal().is_none());
    assert!(restored.cached_view().readiness.is_none());
    Ok(())
}

#[test]
fn partial_joint_decode_and_refused_future_read_retain_consumed_authority_floor() -> Result<()> {
    for variant in 0..3 {
        let (mut clock, _, mut native, mut c, mut n) = setup(1)?;
        let definition = clock.definition().clone();
        let mut sample = evidence(&definition, fixtures::TICK + 30)?;
        match variant {
            0 => sample.after_capture.clear(),
            1 => sample.furnishings.capture.clear(),
            _ => sample.before.generation += 1,
        }
        assert!(
            clock
                .prepare_guarded(&mut native, &request(&definition), &c, &n, |_, _, _| {
                    Ok(RoomClockSample {
                        progress: retained(&definition, &c)?,
                        sample: sample.clone(),
                    })
                })
                .is_err()
        );
        assert_eq!(clock.high_tick(), fixtures::TICK + 30);
        assert_eq!((native.prepares, native.commits), (0, 0));
        for grant in c.grants.iter_mut().chain(n.grants.iter_mut()) {
            grant.expires_at_tick = Some(GameTick(fixtures::TICK + 20));
        }
        c.anchor.tick = GameTick(0);
        n.anchor.tick = GameTick(0);
        assert!(clock.inspect(&c, &n).is_err());
        assert_eq!(clock.cached_view().high_tick, fixtures::TICK + 30);
    }
    Ok(())
}

#[test]
fn query_only_recovery_and_zero_tick_safety_cancel_do_not_repeat_unpause() -> Result<()> {
    let (mut clock, storage, mut native, c, n) = setup(1)?;
    let seal = prepare(&mut clock, &mut native, &c, &n)?;
    native.lose_commit = true;
    assert!(commit(&mut clock, &mut native, seal, &c, &n).is_err());
    let definition = clock.definition().clone();
    let sources = clock.source().clone();
    let pin = clock.recovery();
    drop(clock);
    let mut query = c.clone();
    let mut native_query = n.clone();
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    native_query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    query.budget.max_game_ticks = 0;
    native_query.budget.max_game_ticks = 0;
    let journal = open(
        storage.clone(),
        &native,
        &native_query,
        RunMode::Recover,
        false,
    )?;
    let mut recovered = Clock::new(
        journal,
        definition.clone(),
        sources.clone(),
        Some(pin),
        &query,
        &native_query,
    )?;
    assert!(
        recovered
            .reconcile(&mut native, &query, &native_query)?
            .unresolved
    );
    assert!(recovered.cancel(Some(&mut native), &c, &n).is_err());
    let next_pin = recovered.recovery();
    drop(recovered);
    query.grants = c
        .grants
        .into_iter()
        .filter(|grant| {
            matches!(
                grant.capability,
                Capability::Query | Capability::ControlClock
            )
        })
        .collect();
    native_query.grants = n
        .grants
        .into_iter()
        .filter(|grant| {
            matches!(
                grant.capability,
                Capability::Query | Capability::ControlClock
            )
        })
        .collect();
    let journal = open(storage, &native, &native_query, RunMode::Control, false)?;
    let mut safety = Clock::new(
        journal,
        definition,
        sources,
        Some(next_pin),
        &query,
        &native_query,
    )?;
    assert!(
        safety
            .cancel(Some(&mut native), &query, &native_query)?
            .can_sample_goal()
    );
    assert_eq!((native.commits, native.cancels), (1, 1));
    Ok(())
}

fn admit(_: &Definition, _: &Progress) -> Result<()> {
    Ok(())
}
fn origin_guard(_: &construction::origin::Origin, _: &OperationContext) -> Result<()> {
    Ok(())
}
fn acknowledge(
    monitor: &mut super::super::store::MonitorStore<Memory>,
    sample: Sample,
    context: &OperationContext,
) -> Result<RoomClockSample> {
    monitor.begin_read(context, &mut admit, &mut origin_guard)?;
    monitor.publish_readiness(context, &sample, &mut admit, &mut origin_guard)?;
    Ok(RoomClockSample {
        progress: monitor.progress().clone(),
        sample,
    })
}

#[test]
fn real_intervals_between_ordinary_samples_can_complete_one_original_room_streak() -> Result<()> {
    for count in [1, 32] {
        let (mut clock, _, mut native, c, n) = setup(count)?;
        let definition = clock.definition().clone();
        let mut monitor = super::super::store::MonitorStore::open(
            Memory::default(),
            &c,
            crate::build_placement::journal::BuildMode::Recover,
            Some(definition.clone()),
            true,
        )?;
        monitor.begin_read(&c, &mut admit, &mut origin_guard)?;
        monitor.publish_sample(
            &c,
            &pending(&definition, fixtures::TICK)?,
            &mut admit,
            &mut origin_guard,
        )?;
        assert_eq!(monitor.progress().streak, 0);
        for interval in 1..=2 {
            let view = clock.prepare_guarded(
                &mut native,
                &request(&definition),
                &c,
                &n,
                |_, plan, current| {
                    let sample = evidence(&definition, plan.before().tick().ok_or_else(stale)?)?;
                    acknowledge(&mut monitor, sample, current)
                },
            )?;
            let seal = view
                .active()
                .and_then(|run| run.review_seal)
                .ok_or_else(stale)?;
            assert_eq!(monitor.progress().streak, interval - 1);
            clock.commit_guarded(&mut native, seal, true, &c, &n, |_, plan, current| {
                let sample = evidence(&definition, plan.before().tick().ok_or_else(stale)?)?;
                acknowledge(&mut monitor, sample, current)
            })?;
            assert_eq!(monitor.progress().streak, interval - 1);
            let tick = fixtures::TICK + u64::from(interval) * 10;
            native.finish(tick)?;
            assert!(clock.reconcile(&mut native, &c, &n)?.can_sample_goal());
            monitor.begin_read(&c, &mut admit, &mut origin_guard)?;
            monitor.publish_sample(
                &c,
                &evidence(&definition, tick)?,
                &mut admit,
                &mut origin_guard,
            )?;
            assert_eq!(monitor.progress().streak, interval);
        }
        assert_eq!(monitor.progress().phase, "satisfied");
        assert_eq!(monitor.progress().observations, 7);
        assert_eq!(monitor.progress().condition_met_count(), count);
        assert!(monitor.progress().terrain.condition_met());
        assert_eq!((native.prepares, native.commits, native.queries), (2, 2, 2));
    }
    Ok(())
}

#[test]
fn acknowledged_known_negative_remains_durable_when_later_clock_guard_fails() -> Result<()> {
    let (mut clock, _, mut native, c, n) = setup(1)?;
    let definition = clock.definition().clone();
    let disk = Memory::default();
    let mut monitor = super::super::store::MonitorStore::open(
        disk.clone(),
        &c,
        crate::build_placement::journal::BuildMode::Recover,
        Some(definition.clone()),
        true,
    )?;
    monitor.begin_read(&c, &mut admit, &mut origin_guard)?;
    monitor.publish_sample(
        &c,
        &evidence(&definition, fixtures::TICK)?,
        &mut admit,
        &mut origin_guard,
    )?;
    assert_eq!(monitor.progress().streak, 1);
    assert!(
        clock
            .prepare_guarded(
                &mut native,
                &request(&definition),
                &c,
                &n,
                |_, _, current| {
                    let mut sample = evidence(&definition, fixtures::TICK)?;
                    let wall = definition.room().terrain().walls()[0];
                    change_map(&mut sample, |map| {
                        let index = map.map.region.index(wall).ok_or_else(stale)?;
                        let Cell::Visible(mut tile) = map.map.cells[index] else {
                            return Err(stale());
                        };
                        tile.shape = Shape::Floor;
                        map.map.cells[index] = Cell::Visible(tile);
                        Ok(())
                    })?;
                    acknowledge(&mut monitor, sample, current)?;
                    Err(error(
                        ErrorCode::CapabilityDenied,
                        "late enclosing admission refused",
                    ))
                }
            )
            .is_err()
    );
    assert_eq!(
        (monitor.progress().streak, monitor.progress().observations),
        (0, 2)
    );
    let pin = monitor.recovery();
    drop(monitor);
    let mut recovered = super::super::store::MonitorStore::open(
        disk,
        &c,
        crate::build_placement::journal::BuildMode::Offline,
        Some(definition),
        false,
    )?;
    recovered.verify_recovery(&c, pin)?;
    assert_eq!(
        (
            recovered.progress().streak,
            recovered.progress().observations
        ),
        (0, 2)
    );
    assert_eq!(recovered.progress().terrain.deficit_count(), 1);
    assert_eq!((native.prepares, native.commits), (0, 0));
    Ok(())
}

#[test]
fn readiness_probe_never_uses_the_last_acquisition_or_grants_positive_credit() -> Result<()> {
    let (original, _) = fixture(1)?;
    let mut timing = original.goal().timing().clone();
    timing.max_observations = 3;
    let definition = with_timing(&original, timing)?;
    let (c, _) = contexts(&definition);
    let pin = source_identity(&definition)?;
    let before = retained(&definition, &c)?;
    let sample = evidence(&definition, fixtures::TICK + 10)?;
    let after = super::super::readiness::advance(&before.begin_read()?, &definition, &sample, &c)?;
    assert_eq!(
        (after.observations, after.streak, after.counted_tick),
        (2, 1, before.counted_tick)
    );
    for progress in [&before, &after] {
        let readiness = capture_readiness(&definition, &pin, progress, &sample, &c)?;
        assert_eq!(readiness.complete_count(), 1);
        assert!(!readiness.withdraws_stability());
        assert!(!progress.terminal());
    }
    let exhausted =
        super::super::readiness::advance(&after.begin_read()?, &definition, &sample, &c)?;
    assert_eq!((exhausted.phase, exhausted.observations), ("expired", 3));
    assert!(capture_readiness(&definition, &pin, &exhausted, &sample, &c).is_err());
    Ok(())
}

#[test]
fn current_and_opening_authority_and_room_budget_precede_any_native_contact() -> Result<()> {
    for variant in 0..7 {
        let (mut clock, _, mut native, mut c, mut n) = setup(1)?;
        match variant {
            0 => c.grants.retain(|g| g.capability != Capability::Observe),
            1 => c.grants.retain(|g| g.capability != Capability::Plan),
            2 => c
                .grants
                .retain(|g| g.capability != Capability::ControlClock),
            3 => n.grants.retain(|g| g.capability != Capability::Plan),
            4 => n.grants = c.grants.clone(),
            5 => c.budget.max_game_ticks = 0,
            _ => {
                c.budget.max_bytes = OPERATION_BYTES - 1;
                n.budget.max_bytes = OPERATION_BYTES - 1;
            }
        }
        assert!(prepare(&mut clock, &mut native, &c, &n).is_err());
        // A zero future-tick allowance is checked after the one selected read,
        // but still before durable preparation or any native effect.
        assert_eq!((native.prepares, native.commits), (0, 0));
        if variant != 5 {
            assert_eq!(native.observes, 0);
        }
    }
    let (definition, _) = fixture(1)?;
    let (mut c, n) = contexts(&definition);
    c.grants.retain(|g| g.capability != Capability::Plan);
    let storage = Memory::default();
    let mut native = Source::new(storage.clone());
    let journal = open(storage, &native, &n, RunMode::Control, true)?;
    let mut clock = Clock::new(
        journal,
        definition.clone(),
        source_identity(&definition)?,
        None,
        &c,
        &n,
    )?;
    let (restored, _) = contexts(&definition);
    assert!(prepare(&mut clock, &mut native, &restored, &n).is_err());
    assert_eq!(native.observes, 0);
    Ok(())
}

#[test]
fn acknowledged_recovery_heads_reject_rollback_and_never_restore_a_review() -> Result<()> {
    let (mut clock, storage, mut native, c, n) = setup(1)?;
    let before = clock.recovery();
    let empty = storage.snapshot_bytes();
    let seal = prepare(&mut clock, &mut native, &c, &n)?;
    let after = clock.recovery();
    let definition = clock.definition().clone();
    let sources = clock.source().clone();
    drop(clock);
    let old = open(
        Memory::from_bytes(empty),
        &native,
        &n,
        RunMode::Offline,
        false,
    )?;
    assert!(
        Clock::new(
            old,
            definition.clone(),
            sources.clone(),
            Some(after),
            &c,
            &n
        )
        .is_err()
    );
    let journal = open(storage, &native, &n, RunMode::Control, false)?;
    let mut recovered = Clock::new(journal, definition, sources, Some(before), &c, &n)?;
    assert_eq!(recovered.recovery(), after);
    assert!(recovered.prepared_seal().is_none());
    assert!(commit(&mut recovered, &mut native, seal, &c, &n).is_err());
    assert!(recovered.cancel(None, &c, &n)?.can_sample_goal());
    assert_eq!((native.commits, native.cancels), (0, 0));
    Ok(())
}

#[test]
fn changed_room_source_or_clock_bytes_at_commit_cannot_unpause() -> Result<()> {
    for variant in 0..2 {
        let (mut clock, storage, mut native, c, n) = setup(1)?;
        let definition = clock.definition().clone();
        let seal = prepare(&mut clock, &mut native, &c, &n)?;
        assert!(
            clock
                .commit_guarded(&mut native, seal, true, &c, &n, |_, plan, current| {
                    let mut capture = ready(
                        &definition,
                        plan.before().tick().ok_or_else(stale)?,
                        current,
                    )?;
                    if variant == 0 {
                        capture.sample.after.generation += 1;
                    } else {
                        storage.damage();
                    }
                    Ok(capture)
                })
                .is_err()
        );
        assert_eq!(native.commits, 0);
        assert!(clock.prepared_seal().is_none());
        assert!(commit(&mut clock, &mut native, seal, &c, &n).is_err());
        assert_eq!(native.commits, 0);
    }
    Ok(())
}
