use super::*;
use crate::bounded_run::RunPhase;
use crate::construction_plan::store::tests::fixture;
use crate::construction_plan::{Timing, fixtures};
use crate::live_jobs::LiveJob;
use dfmcp_core::{CapabilityScope, MapCoord, MapCuboid, RequestId};
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

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

fn contexts(spec: &MonitorDefinition) -> (OperationContext, OperationContext) {
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
fn operations() -> Manifest {
    Manifest {
        generation: 987,
        df_version: "test-df".into(),
        dfhack_version: "test-dfhack".into(),
    }
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
fn setup() -> Result<(
    Clock<Memory>,
    Memory,
    Source,
    OperationContext,
    OperationContext,
)> {
    with_spec(fixture()?.definition)
}
fn with_spec(
    spec: MonitorDefinition,
) -> Result<(
    Clock<Memory>,
    Memory,
    Source,
    OperationContext,
    OperationContext,
)> {
    let (c, n) = contexts(&spec);
    let storage = Memory::default();
    let source = Source::new(storage.clone());
    let journal = open(storage.clone(), &source, &n, RunMode::Control, true)?;
    let clock = Clock::new(journal, spec, operations(), None, &c, &n)?;
    Ok((clock, storage, source, c, n))
}
fn request(spec: &MonitorDefinition) -> RunRequest {
    RunRequest {
        monitor_id: spec.id(),
        game_ticks: 10,
        wall_millis: 500,
    }
}
fn pending(spec: &MonitorDefinition, tick: u64) -> Result<LinkedSample> {
    let mut observed = fixtures::observation(spec.goal(), tick);
    let p = spec.goal().records()[0].insertion().ok_or_else(stale)?;
    observed.buildings[0].build_stage = p.max_stage() as i32 - 1;
    observed.jobs.jobs.push(LiveJob {
        native_id: p.job_id(),
        job_type: 1,
        type_key: "ConstructBuilding".into(),
        reaction: String::new(),
        suspended: false,
        repeating: false,
        position: MapCoord::new(15, 15, 2),
        worker_native_id: None,
        holder_native_id: Some(p.building_id()),
        completion_timer: -1,
        attached_item_count: 0,
        required_item_filter_count: 0,
    });
    Ok(fixtures::with_observation(spec.goal(), observed))
}
fn retained(spec: &MonitorDefinition) -> Result<Progress> {
    let (c, _) = contexts(spec);
    super::super::advance(
        &Progress::new(spec.goal()).begin_read()?,
        spec.goal(),
        &pending(spec, fixtures::TICK)?,
        &c,
    )
}
fn ready(spec: &MonitorDefinition, tick: u64) -> Result<ClockSample> {
    Ok(ClockSample {
        progress: retained(spec)?,
        sample: pending(spec, tick)?,
    })
}
fn prepare(
    clock: &mut Clock<Memory>,
    source: &mut Source,
    c: &OperationContext,
    n: &OperationContext,
) -> Result<Digest32> {
    let spec = clock.spec().clone();
    let view = clock.prepare_guarded(source, &request(&spec), c, n, |edge, p, _| {
        assert_eq!(edge, Edge::Prepare);
        ready(&spec, p.before().tick().ok_or_else(stale)?)
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
    let spec = clock.spec().clone();
    clock.commit_guarded(source, seal, true, c, n, |edge, p, _| {
        assert_eq!(edge, Edge::Commit);
        ready(&spec, p.before().tick().ok_or_else(stale)?)
    })
}
fn changed_timing(spec: &MonitorDefinition, timing: Timing) -> Result<MonitorDefinition> {
    MonitorDefinition::new(
        spec.origin().clone(),
        super::super::Goal::new(spec.goal().records().to_vec(), timing)?,
    )
}
fn code<T>(result: Result<T>) -> Option<ErrorCode> {
    result.err().map(|error| error.code)
}

#[test]
fn external_guards_run_after_synced_intent_and_dispatch_marker() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let spec = clock.spec().clone();
    let mut edges = Vec::new();
    let view = clock.prepare_guarded(&mut source, &request(&spec), &c, &n, |edge, p, _| {
        edges.push(edge);
        let mut reopened = RunJournal::open(storage.clone(), &n, RunMode::Offline, None, false)?;
        assert_eq!(reopened.records(&n)?[0].state(), RunState::Intent);
        ready(&spec, p.before().tick().ok_or_else(stale)?)
    })?;
    let seal = view
        .active()
        .and_then(|run| run.review_seal)
        .ok_or_else(stale)?;
    clock.commit_guarded(&mut source, seal, true, &c, &n, |edge, p, _| {
        edges.push(edge);
        let mut reopened = RunJournal::open(storage.clone(), &n, RunMode::Offline, None, false)?;
        assert_eq!(reopened.records(&n)?[0].state(), RunState::DispatchStarted);
        ready(&spec, p.before().tick().ok_or_else(stale)?)
    })?;
    assert_eq!(edges, vec![Edge::Prepare, Edge::Commit]);
    assert_eq!((source.prepares, source.commits), (1, 1));
    Ok(())
}

#[test]
fn same_size_clock_corruption_during_either_guard_blocks_native_boundary() -> Result<()> {
    for at_commit in [false, true] {
        let (mut clock, storage, mut source, c, n) = setup()?;
        let spec = clock.spec().clone();
        if at_commit {
            let seal = prepare(&mut clock, &mut source, &c, &n)?;
            assert_eq!(
                code(
                    clock.commit_guarded(&mut source, seal, true, &c, &n, |_, p, _| {
                        storage.damage();
                        ready(&spec, p.before().tick().ok_or_else(stale)?)
                    })
                ),
                Some(ErrorCode::EffectIndeterminate)
            );
            assert_eq!(source.commits, 0);
            assert_eq!(
                clock.cached_view().runs[0].entry.state(),
                RunState::DispatchStarted
            );
        } else {
            assert!(
                clock
                    .prepare_guarded(&mut source, &request(&spec), &c, &n, |_, p, _| {
                        storage.damage();
                        ready(&spec, p.before().tick().ok_or_else(stale)?)
                    })
                    .is_err()
            );
            assert_eq!(source.prepares, 0);
        }
        assert!(clock.cached_view().unresolved);
        assert!(clock.prepared_seal().is_none());
        assert!(clock.inspect(&c, &n).is_err());
    }
    Ok(())
}

#[test]
fn rejected_post_marker_original_guard_never_restores_permission() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    let pin = clock.recovery();
    assert_eq!(
        code(clock.commit_guarded(&mut source, seal, true, &c, &n, |_, _, _| Err(stale()))),
        Some(ErrorCode::EffectIndeterminate)
    );
    assert_eq!(source.commits, 0);
    let spec = clock.spec().clone();
    drop(clock);
    let journal = open(storage, &source, &n, RunMode::Control, false)?;
    let mut resumed = Clock::new(journal, spec, operations(), Some(pin), &c, &n)?;
    assert!(resumed.prepared_seal().is_none());
    let queried = resumed.reconcile(&mut source, &c, &n)?;
    assert_eq!(
        queried.active().map(|run| run.entry.state()),
        Some(RunState::Tracking)
    );
    assert_eq!(
        queried
            .active()
            .and_then(|run| run.entry.native())
            .map(RunRecord::phase),
        Some(RunPhase::Prepared)
    );
    assert!(commit(&mut resumed, &mut source, seal, &c, &n).is_err());
    resumed.cancel(Some(&mut source), &c, &n)?;
    assert_eq!((source.commits, source.cancels), (0, 1));
    Ok(())
}

#[test]
fn lost_commit_reply_and_absent_query_never_allow_redispatch() -> Result<()> {
    let (mut clock, _, mut source, c, n) = setup()?;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    source.lose_commit = true;
    assert_eq!(
        code(commit(&mut clock, &mut source, seal, &c, &n)),
        Some(ErrorCode::EffectIndeterminate)
    );
    source.absent = true;
    assert!(clock.reconcile(&mut source, &c, &n).is_err());
    assert!(clock.inspect(&c, &n)?.unresolved);
    assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
    assert!(commit(&mut clock, &mut source, seal, &c, &n).is_err());
    source.absent = false;
    source.finish(fixtures::TICK + 11)?;
    assert!(!clock.reconcile(&mut source, &c, &n)?.pending);
    assert_eq!(source.commits, 1);
    Ok(())
}

#[test]
fn prepared_reopen_requires_exact_pin_and_new_process_cannot_commit_old_review() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    let pin = clock.recovery();
    let spec = clock.spec().clone();
    drop(clock);
    let journal = open(storage.clone(), &source, &n, RunMode::Control, false)?;
    assert!(Clock::new(journal, spec.clone(), operations(), None, &c, &n).is_err());
    let journal = open(storage.clone(), &source, &n, RunMode::Control, false)?;
    let mut resumed = Clock::new(journal, spec, operations(), Some(pin), &c, &n)?;
    assert!(resumed.inspect(&c, &n)?.runs[0].review_seal.is_none());
    assert!(commit(&mut resumed, &mut source, seal, &c, &n).is_err());
    assert!(resumed.cancel(None, &c, &n)?.runs[0].entry.terminal());
    assert_eq!((source.commits, source.cancels), (0, 0));
    Ok(())
}

#[test]
fn external_head_allows_complete_descendants_but_rejects_acknowledged_rollback() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let original = clock.recovery();
    let initial_bytes = storage.snapshot_bytes();
    let spec = clock.spec().clone();
    prepare(&mut clock, &mut source, &c, &n)?;
    let acknowledged = clock.recovery();
    let descendant_bytes = storage.snapshot_bytes();
    for pin in [original, acknowledged] {
        let raw = Memory::from_bytes(descendant_bytes.clone());
        let journal = open(raw, &source, &n, RunMode::Offline, false)?;
        let resumed = Clock::new(journal, spec.clone(), operations(), Some(pin), &c, &n)?;
        assert_eq!(resumed.recovery(), acknowledged);
    }
    for pin in [
        acknowledged,
        Recovery {
            journal_id: original.journal_id,
            head: Digest32::of_bytes(b"foreign"),
        },
        Recovery {
            journal_id: Digest32::ZERO,
            head: original.head,
        },
    ] {
        let journal = open(
            Memory::from_bytes(initial_bytes.clone()),
            &source,
            &n,
            RunMode::Offline,
            false,
        )?;
        assert!(Clock::new(journal, spec.clone(), operations(), Some(pin), &c, &n).is_err());
    }
    let journal = open(
        Memory::from_bytes(initial_bytes.clone()),
        &source,
        &n,
        RunMode::Offline,
        false,
    )?;
    assert!(Clock::new(journal, spec.clone(), operations(), None, &c, &n).is_err());
    let journal = open(
        Memory::from_bytes(initial_bytes),
        &source,
        &n,
        RunMode::Offline,
        false,
    )?;
    assert!(Clock::new(journal, spec, operations(), Some(original), &c, &n).is_ok());
    Ok(())
}

#[test]
fn rejected_future_construction_retains_tick_and_cannot_revive_original_expired_authority()
-> Result<()> {
    let (mut clock, _, mut source, mut c, mut n) = setup()?;
    let spec = clock.spec().clone();
    for grant in &mut c.grants {
        grant.expires_at_tick = Some(GameTick(fixtures::TICK + 20));
    }
    for grant in &mut n.grants {
        grant.expires_at_tick = Some(GameTick(fixtures::TICK + 20));
    }
    // Make opening authority independently finite, then supply an apparent renewal later.
    clock.authority.canonical = c.grants.clone();
    clock.authority.native = n.grants.clone();
    assert!(
        clock
            .prepare_guarded(&mut source, &request(&spec), &c, &n, |_, _, _| ready(
                &spec,
                fixtures::TICK + 30
            ))
            .is_err()
    );
    assert_eq!(clock.high_tick(), fixtures::TICK + 30);
    assert_eq!(source.prepares, 0);
    for grant in &mut c.grants {
        grant.expires_at_tick = None;
    }
    for grant in &mut n.grants {
        grant.expires_at_tick = None;
    }
    c.anchor.tick = GameTick(0);
    n.anchor.tick = GameTick(0);
    assert_eq!(
        code(clock.inspect(&c, &n)),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(clock.cached_view().high_tick, fixtures::TICK + 30);
    Ok(())
}

#[test]
fn current_and_original_canonical_and_independent_native_scopes_are_all_required() -> Result<()> {
    for variant in 0..8 {
        let (mut clock, _, mut source, mut c, mut n) = setup()?;
        match variant {
            0 => c.grants.retain(|g| g.capability != Capability::Observe),
            1 => c.grants.retain(|g| g.capability != Capability::Plan),
            2 => {
                let area = MapCuboid::new(MapCoord::new(1, 1, 1), MapCoord::new(2, 2, 1))?;
                for grant in c
                    .grants
                    .iter_mut()
                    .filter(|g| g.capability == Capability::ControlClock)
                {
                    grant.scope.map_area = Some(area);
                }
            }
            3 => n.grants.retain(|g| g.capability != Capability::Plan),
            4 => n.anchor.fortress_id = c.anchor.fortress_id,
            5 => n.request_id = RequestId::new(900),
            6 => clock
                .authority
                .native
                .retain(|g| g.capability != Capability::ControlClock),
            _ => clock
                .authority
                .canonical
                .retain(|g| g.capability != Capability::Plan),
        }
        assert!(
            prepare(&mut clock, &mut source, &c, &n).is_err(),
            "variant {variant}"
        );
        assert_eq!(
            (source.observes, source.prepares, source.commits),
            (0, 0, 0)
        );
    }
    Ok(())
}

#[test]
fn local_or_native_stop_survives_plan_and_observe_revocation_and_elapsed_goal() -> Result<()> {
    for running in [false, true] {
        let (mut clock, _, mut source, mut c, mut n) = setup()?;
        let seal = prepare(&mut clock, &mut source, &c, &n)?;
        if running {
            commit(&mut clock, &mut source, seal, &c, &n)?;
        }
        c.grants
            .retain(|g| matches!(g.capability, Capability::Query | Capability::ControlClock));
        n.grants
            .retain(|g| matches!(g.capability, Capability::Query | Capability::ControlClock));
        c.budget.max_game_ticks = 0;
        n.budget.max_game_ticks = 0;
        c.anchor.tick = GameTick(fixtures::TICK + 2000);
        n.anchor.tick = GameTick(fixtures::TICK + 2000);
        source.tick = fixtures::TICK + 2000;
        let view = clock.cancel(if running { Some(&mut source) } else { None }, &c, &n)?;
        assert!(!view.pending);
        assert_eq!(source.cancels, usize::from(running));
        assert!(clock.prepared_seal().is_none());
    }
    Ok(())
}

#[test]
fn source_change_and_source_lost_remain_unresolved_without_proving_current_pause() -> Result<()> {
    let (mut clock, _, mut source, c, n) = setup()?;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    commit(&mut clock, &mut source, seal, &c, &n)?;
    let plan = source.native.as_ref().ok_or_else(corrupt)?.plan().clone();
    source.manifest.generation += 1;
    source.native = Some(record(&plan, 5, 7, 0)?);
    let lost = clock.reconcile(&mut source, &c, &n)?;
    assert!(lost.pending && lost.unresolved && !lost.can_sample_goal());
    assert!(!lost.needs_reconciliation());
    assert!(
        lost.runs[0]
            .entry
            .native()
            .is_some_and(|value| !value.pause_verified())
    );
    assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
    assert_eq!(source.commits, 1);
    Ok(())
}

#[test]
fn lost_receipt_sync_fences_but_complete_descendant_recovers_exact_evidence() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    let pin = clock.recovery();
    let spec = clock.spec().clone();
    source.fail_receipt_sync = true;
    assert_eq!(
        code(commit(&mut clock, &mut source, seal, &c, &n)),
        Some(ErrorCode::EffectIndeterminate)
    );
    assert_eq!(source.commits, 1);
    assert_eq!(clock.high_sequence(), 4);
    assert!(clock.cached_view().unresolved);
    assert!(clock.prepared_seal().is_none());
    drop(clock);
    let io_counts = storage.operations();
    let journal = open(storage.clone(), &source, &n, RunMode::Offline, false)?;
    let mut recovered = Clock::new(journal, spec, operations(), Some(pin), &c, &n)?;
    let current = recovered.inspect(&c, &n)?;
    assert_eq!(current.runs[0].entry.state(), RunState::Tracking);
    assert_eq!(
        current.runs[0].entry.native().map(RunRecord::phase),
        Some(RunPhase::Running)
    );
    assert_eq!(io_counts, storage.operations());
    assert!(recovered.reconcile(&mut source, &c, &n).is_err());
    assert_eq!(source.queries, 0);
    Ok(())
}

#[test]
fn query_only_recovery_and_offline_mode_never_acquire_new_control() -> Result<()> {
    let (mut clock, storage, mut source, mut c, mut n) = setup()?;
    prepare(&mut clock, &mut source, &c, &n)?;
    let pin = clock.recovery();
    let spec = clock.spec().clone();
    drop(clock);
    c.grants.retain(|g| g.capability == Capability::Query);
    n.grants.retain(|g| g.capability == Capability::Query);
    for mode in [RunMode::Recover, RunMode::Offline] {
        let journal = open(storage.clone(), &source, &n, mode, false)?;
        let mut owner = Clock::new(journal, spec.clone(), operations(), Some(pin), &c, &n)?;
        assert!(owner.inspect(&c, &n)?.pending);
        let (renewed_c, renewed_n) = contexts(&spec);
        assert!(prepare(&mut owner, &mut source, &renewed_c, &renewed_n).is_err());
        assert!(owner.cancel(None, &renewed_c, &renewed_n).is_err());
        assert!(owner.prepared_seal().is_none());
    }
    assert_eq!((source.prepares, source.commits, source.cancels), (1, 0, 0));
    Ok(())
}

#[test]
fn later_session_floor_rejects_old_native_clock_and_survives_expired_query() -> Result<()> {
    let spec = fixture()?.definition;
    let (mut c, mut n) = contexts(&spec);
    for grant in c.grants.iter_mut().chain(n.grants.iter_mut()) {
        grant.expires_at_tick = Some(GameTick(fixtures::TICK + 500));
    }
    let storage = Memory::default();
    let mut source = Source::new(storage.clone());
    let journal = open(storage, &source, &n, RunMode::Control, true)?;
    let mut clock = Clock::new(journal, spec, operations(), None, &c, &n)?;
    c.anchor.tick = GameTick(fixtures::TICK + 300);
    n.anchor.tick = GameTick(fixtures::TICK + 300);
    assert_eq!(
        code(prepare(&mut clock, &mut source, &c, &n)),
        Some(ErrorCode::StaleAnchor)
    );
    assert_eq!(clock.high_tick(), fixtures::TICK + 300);
    assert_eq!(source.prepares, 0);
    assert!(clock.cached_view().runs.is_empty());

    c.anchor.tick = GameTick(fixtures::TICK + 600);
    n.anchor.tick = GameTick(fixtures::TICK + 600);
    assert_eq!(
        code(clock.inspect(&c, &n)),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(clock.high_tick(), fixtures::TICK + 600);
    c.anchor.tick = GameTick(0);
    n.anchor.tick = GameTick(0);
    for grant in c.grants.iter_mut().chain(n.grants.iter_mut()) {
        grant.expires_at_tick = None;
    }
    assert_eq!(
        code(clock.inspect(&c, &n)),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(source.prepares, 0);
    Ok(())
}

#[test]
fn complete_original_furniture_set_advances_then_requires_independent_completion_samples()
-> Result<()> {
    let (mut clock, _, mut source, c, n) = setup()?;
    assert_eq!(clock.operations().generation, 987);
    assert_eq!(clock.binding().manifest().generation, 41);
    // Independent incarnations may coincidentally share a numeric value; only
    // their own source pins determine equality across time.
    assert_ne!(
        clock.operations().generation,
        clock.binding().manifest().generation
    );
    let old = retained(clock.spec())?;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    let prepared = clock.inspect(&c, &n)?;
    assert!(prepared.pending && !prepared.can_sample_goal());
    let readiness = prepared.readiness.as_ref().ok_or_else(stale)?;
    assert_eq!(
        (readiness.pending_count(), readiness.complete_count()),
        (1, 1)
    );
    assert_eq!(readiness.tick(), fixtures::TICK);
    let running = commit(&mut clock, &mut source, seal, &c, &n)?;
    assert!(running.pending && running.unresolved && !running.can_sample_goal());
    assert!(running.readiness.is_some());
    assert!(clock.prepared_seal().is_none());
    source.finish(fixtures::TICK + 10)?;
    let stopped = clock.reconcile(&mut source, &c, &n)?;
    assert!(!stopped.pending && stopped.can_sample_goal());
    assert_eq!(stopped.high_tick, fixtures::TICK + 10);
    assert_eq!(stopped.high_sequence, 4);
    assert!(
        stopped.runs[0]
            .entry
            .native()
            .is_some_and(RunRecord::pause_verified)
    );
    assert_eq!((source.prepares, source.commits), (1, 1));
    // Readiness and native pause evidence never mutate the monitor's progress.
    assert_eq!(retained(clock.spec())?, old);
    assert_eq!(old.observations, 1);
    assert_eq!(old.streak, 0);
    assert!(clock.cached_view().readiness.is_none());
    Ok(())
}

#[test]
fn readiness_requires_every_original_receipt_and_current_viable_target() -> Result<()> {
    let spec = fixture()?.definition;
    let (c, _) = contexts(&spec);
    let progress = retained(&spec)?;
    let raw = pending(&spec, fixtures::TICK)?;
    let result = Readiness::capture(&spec, &operations(), &progress, &raw, &c)?;
    assert_eq!((result.pending_count(), result.complete_count()), (1, 1));
    for variant in 0..10 {
        let mut sample = raw.clone();
        let mut observed = decode_capture(&sample)?;
        match variant {
            0 => {
                sample.before_records.pop();
            }
            1 => {
                sample.after_records[1][0] ^= 1;
            }
            2 => {
                sample.operations.generation += 1;
            }
            3 => {
                observed.jobs.paused = false;
            }
            4 => {
                let removed = observed.buildings.pop().ok_or_else(stale)?;
                for item in &mut observed.items {
                    if item.holder_building_native_id == Some(removed.native_id) {
                        item.holder_building_native_id = None;
                    }
                }
            }
            5 => {
                observed.items.pop();
            }
            6 => {
                observed.jobs.jobs[0].type_key = "DestroyBuilding".into();
            }
            7 => {
                observed.jobs.jobs[0].suspended = true;
            }
            8 => {
                observed.jobs.jobs.clear();
            }
            _ => {
                observed.items[1].material_index += 1;
            }
        }
        if variant >= 3 {
            sample = fixtures::with_observation(spec.goal(), observed);
        }
        assert!(
            Readiness::capture(&spec, &operations(), &progress, &sample, &c).is_err(),
            "variant {variant}"
        );
    }
    Ok(())
}

#[test]
fn readiness_cannot_start_without_an_accepted_source_or_renew_terminal_monitor_limits() -> Result<()>
{
    let spec = fixture()?.definition;
    let (c, _) = contexts(&spec);
    let original = retained(&spec)?;
    let raw = pending(&spec, fixtures::TICK)?;
    for variant in 0..6 {
        let mut progress = original.clone();
        match variant {
            0 => progress = Progress::new(spec.goal()),
            1 => progress.source = None,
            2 => progress.phase = "satisfied",
            3 => progress.phase = "cancelled",
            4 => progress.observations = spec.goal().timing().max_observations,
            _ => progress.reading = true,
        }
        assert!(Readiness::capture(&spec, &operations(), &progress, &raw, &c).is_err());
    }
    let mut final_sample = original;
    final_sample.observations = spec.goal().timing().max_observations - 1;
    // A nonpublished readiness capture does not steal the final real sample.
    assert!(Readiness::capture(&spec, &operations(), &final_sample, &raw, &c).is_ok());
    assert_eq!(
        final_sample.observations,
        spec.goal().timing().max_observations - 1
    );
    Ok(())
}

#[test]
fn already_prospectively_satisfied_goal_refuses_unnecessary_unpause() -> Result<()> {
    let (mut clock, _, mut source, c, n) = setup()?;
    let spec = clock.spec().clone();
    let initial = fixtures::sample(spec.goal(), fixtures::TICK);
    let progress = super::super::advance(
        &Progress::new(spec.goal()).begin_read()?,
        spec.goal(),
        &initial,
        &c,
    )?;
    assert_eq!(progress.streak, 1);
    source.tick += 10;
    let raw = fixtures::sample(spec.goal(), source.tick);
    assert!(
        clock
            .prepare_guarded(&mut source, &request(&spec), &c, &n, |_, _, _| {
                Ok(ClockSample {
                    progress: progress.clone(),
                    sample: raw.clone(),
                })
            })
            .is_err()
    );
    assert_eq!((source.prepares, source.commits), (0, 0));
    assert_eq!(clock.high_tick(), fixtures::TICK + 10);
    assert_eq!(clock.cached_view().runs[0].entry.state(), RunState::Intent);
    assert_eq!(progress.streak, 1);
    Ok(())
}

#[test]
fn accepted_negative_readiness_explicitly_withdraws_whole_plan_stability() -> Result<()> {
    let spec = fixture()?.definition;
    let (c, _) = contexts(&spec);
    let initial = fixtures::sample(spec.goal(), fixtures::TICK);
    let progress = super::super::advance(
        &Progress::new(spec.goal()).begin_read()?,
        spec.goal(),
        &initial,
        &c,
    )?;
    assert_eq!(progress.streak, 1);
    let mut sample = pending(&spec, fixtures::TICK)?;
    let mut observed = decode_capture(&sample)?;
    // A newly present original job contradicts completion without a stage regression.
    observed.buildings[0].build_stage = observed.buildings[0].max_build_stage;
    sample = fixtures::with_observation(spec.goal(), observed);
    let pending = Readiness::capture(&spec, &operations(), &progress, &sample, &c)?;
    assert_eq!(pending.pending_count(), 1);
    assert!(pending.withdraws_stability());
    let mut observed = decode_capture(&initial)?;
    observed.items[0].raw_position.x += 1;
    let changed = fixtures::with_observation(spec.goal(), observed);
    let correction = Readiness::capture(&spec, &operations(), &progress, &changed, &c)?;
    assert_eq!(correction.complete_count(), 2);
    assert!(correction.withdraws_stability());
    assert_eq!(progress.streak, 1);
    Ok(())
}

#[test]
fn complete_definition_and_independent_operations_pin_are_bound_to_existing_run_keys() -> Result<()>
{
    let (mut clock, storage, mut source, c, n) = setup()?;
    prepare(&mut clock, &mut source, &c, &n)?;
    let pin = clock.recovery();
    let spec = clock.spec().clone();
    for changed_source in [false, true] {
        let journal = open(storage.clone(), &source, &n, RunMode::Offline, false)?;
        let mut generation = operations();
        let mut definition = spec.clone();
        if changed_source {
            generation.generation += 1;
        } else {
            definition = changed_timing(
                &spec,
                Timing {
                    deadline: spec.goal().timing().deadline + 1,
                    ..spec.goal().timing()
                },
            )?;
        }
        assert_eq!(
            code(Clock::new(
                journal,
                definition,
                generation,
                Some(pin),
                &c,
                &n
            )),
            Some(ErrorCode::CorruptLedger)
        );
    }
    let reduced =
        super::super::Goal::new(spec.goal().records()[..1].to_vec(), spec.goal().timing())?;
    assert!(MonitorDefinition::new(spec.origin().clone(), reduced).is_err());
    Ok(())
}

#[test]
fn fixed_deadline_gap_allowance_and_retained_run_cap_never_renew() -> Result<()> {
    let original = fixture()?.definition;
    let spec = changed_timing(
        &original,
        Timing {
            deadline: fixtures::TICK + 40,
            max_gap: 10,
            max_observations: 2,
            ..original.goal().timing()
        },
    )?;
    let (mut clock, _, mut source, c, n) = with_spec(spec)?;
    let mut excessive = request(clock.spec());
    excessive.game_ticks = 11;
    let spec = clock.spec().clone();
    assert!(
        clock
            .prepare_guarded(&mut source, &excessive, &c, &n, |_, p, _| ready(
                &spec,
                p.before().tick().ok_or_else(stale)?
            ))
            .is_err()
    );
    assert_eq!(source.prepares, 0);
    for _ in 0..2 {
        let seal = prepare(&mut clock, &mut source, &c, &n)?;
        commit(&mut clock, &mut source, seal, &c, &n)?;
        source.finish(source.tick + 10)?;
        clock.reconcile(&mut source, &c, &n)?;
    }
    assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
    assert_eq!(source.prepares, 2);
    let (mut clock, _, mut source, c, n) = setup()?;
    clock.authority.game_ticks = 5;
    assert_eq!(
        code(prepare(&mut clock, &mut source, &c, &n)),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(source.prepares, 0);
    let (mut clock, _, mut source, c, n) = setup()?;
    source.tick = clock.spec().goal().timing().deadline - 5;
    assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
    assert_eq!(source.prepares, 0);
    Ok(())
}

#[test]
fn source_endpoint_software_and_prepaid_work_are_required_before_control() -> Result<()> {
    let spec = fixture()?.definition;
    let (c, n) = contexts(&spec);
    for variant in 0..3 {
        let storage = Memory::default();
        let mut source = Source::new(storage.clone());
        let mut operations = operations();
        match variant {
            0 => source.endpoint.set_port(5001),
            1 => source.manifest.df_version.push_str("-other"),
            _ => operations.dfhack_version.push_str("-other"),
        }
        let journal = open(storage, &source, &n, RunMode::Control, true)?;
        assert_eq!(
            code(Clock::new(journal, spec.clone(), operations, None, &c, &n)),
            Some(ErrorCode::StaleAnchor)
        );
    }
    let (mut clock, _, mut source, mut c, mut n) = setup()?;
    c.budget.max_bytes = OPERATION_BYTES - 1;
    n.budget.max_bytes = OPERATION_BYTES - 1;
    assert_eq!(
        code(prepare(&mut clock, &mut source, &c, &n)),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(
        (source.observes, source.prepares, source.commits),
        (0, 0, 0)
    );
    c.budget.max_bytes = OPERATION_BYTES;
    n.budget.max_bytes = OPERATION_BYTES;
    assert!(prepare(&mut clock, &mut source, &c, &n).is_ok());
    Ok(())
}

#[test]
fn cached_disclosure_checks_current_and_retained_query_without_touching_custody() -> Result<()> {
    for variant in 0..4 {
        let (mut clock, storage, _, mut c, mut n) = setup()?;
        let before = storage.operations();
        assert!(clock.authorize_disclosure(&c, &n).is_ok());
        let grants = match variant {
            0 => &mut c.grants,
            1 => &mut n.grants,
            2 => &mut clock.authority.canonical,
            _ => &mut clock.authority.native,
        };
        for grant in grants
            .iter_mut()
            .filter(|grant| grant.capability == Capability::Query)
        {
            grant.expires_at_tick = Some(GameTick(fixtures::TICK + 5));
        }
        c.anchor.tick = GameTick(fixtures::TICK + 10);
        n.anchor.tick = c.anchor.tick;
        assert_eq!(
            code(clock.authorize_disclosure(&c, &n)),
            Some(ErrorCode::CapabilityDenied)
        );
        assert_eq!(before, storage.operations());
    }
    Ok(())
}

#[test]
fn final_preparation_publication_expiry_withdraws_seal_without_repeating_dispatch() -> Result<()> {
    let (mut clock, _, mut source, c, n) = setup()?;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    let mut work = Work::new(c.clone(), n.clone(), OPERATION_BYTES)?;
    let output = work.take(VIEW_BYTES, true)?;
    work.deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .ok_or_else(bounded)?;

    assert_eq!(
        code(clock.publish_prepared(&work, &output)),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(clock.prepared_seal(), None);
    assert!(clock.cached_view().readiness.is_none());
    let retained = clock.inspect(&c, &n)?;
    assert_eq!(
        retained.active().map(|run| run.entry.state()),
        Some(RunState::Prepared)
    );
    assert!(commit(&mut clock, &mut source, seal, &c, &n).is_err());
    assert_eq!(source.commits, 0);
    clock.cancel(None, &c, &n)?;
    assert_eq!(source.commits, 0);
    Ok(())
}
