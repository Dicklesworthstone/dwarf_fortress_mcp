use super::*;
use crate::bounded_run::RunPhase;
use crate::dig_blueprint::monitor::tests as fixtures;
use crate::dig_blueprint::monitor::{Policy, SourcePin};
use dfmcp_core::{CapabilityScope, RequestId};
use dfmcp_world::map_region::Tile;
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
            tick: 1000,
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

fn contexts(spec: &MonitorSpec) -> (OperationContext, OperationContext) {
    let mut c = fixtures::context(spec);
    c.budget.max_game_ticks = 1200;
    for capability in [Capability::Plan, Capability::ControlClock] {
        c.grants.push(CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(c.anchor.fortress_id),
                map_area: (capability == Capability::Plan).then(|| spec.parent().binding().scope()),
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
    with_spec(fixtures::spec(fixtures::parts())?)
}
fn with_spec(
    spec: MonitorSpec,
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
    let clock = Clock::new(journal, spec, None, &c, &n)?;
    Ok((clock, storage, source, c, n))
}
fn request(spec: &MonitorSpec) -> RunRequest {
    RunRequest {
        monitor_id: spec.id(),
        game_ticks: 10,
        wall_millis: 500,
    }
}
fn ready(spec: &MonitorSpec, tick: u64) -> Result<LiveMapObservation> {
    let mut value = fixtures::capture(spec, tick)?;
    let position = spec.parent().layout().targets()[0];
    let index = value.map.region.index(position).ok_or_else(stale)?;
    if let Cell::Visible(tile) = &mut value.map.cells[index] {
        tile.shape = Shape::Wall;
        tile.dig_designation = 1;
    }
    Ok(value)
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
fn changed_policy(original: &MonitorSpec, policy: Policy) -> Result<MonitorSpec> {
    let (parent, view, records) =
        fixtures::native_history(original.parent().layout().parts().to_vec())?;
    MonitorSpec::new(
        parent,
        &view,
        &records,
        original.source().clone(),
        policy,
        original.start_tick(),
    )
}
fn code<T>(result: Result<T>) -> Option<ErrorCode> {
    result.err().map(|error| error.code)
}

#[test]
fn sparse_readiness_uses_visible_semantic_dry_original_targets_only() -> Result<()> {
    let spec = fixtures::spec(fixtures::parts())?;
    let raw = ready(&spec, 1000)?;
    let result = Readiness::capture(&spec, raw.clone())?;
    assert_eq!((result.pending_tiles(), result.cleared_tiles()), (1, 1));
    let selected = raw
        .map
        .region
        .index(spec.parent().layout().targets()[0])
        .ok_or_else(stale)?;
    let mut gaps = raw.clone();
    for (index, cell) in gaps.map.cells.iter_mut().enumerate() {
        let position = gaps.map.region.position(index).ok_or_else(stale)?;
        if !spec.parent().layout().targets().contains(&position) {
            *cell = Cell::Unallocated;
        }
    }
    assert!(Readiness::capture(&spec, gaps).is_ok());
    let Cell::Visible(tile) = raw.map.cells[selected] else {
        return Err(stale());
    };
    let mut invalid = vec![Cell::Unallocated, Cell::Hidden];
    for altered in [
        Tile {
            liquid_depth: 1,
            ..tile
        },
        Tile {
            magma: true,
            liquid_depth: 1,
            ..tile
        },
        Tile {
            building_occupancy: 1,
            ..tile
        },
        Tile {
            dig_designation: 0,
            ..tile
        },
        Tile {
            dig_designation: 2,
            ..tile
        },
        Tile {
            shape: Shape::Empty,
            ..tile
        },
        Tile {
            shape: Shape::Floor,
            ..tile
        },
        Tile {
            shape: Shape::Other,
            ..tile
        },
    ] {
        invalid.push(Cell::Visible(altered));
    }
    for cell in invalid {
        let mut changed = raw.clone();
        changed.map.cells[selected] = cell;
        assert!(Readiness::capture(&spec, changed).is_err());
    }
    let mut changed = raw.clone();
    changed.paused = false;
    assert!(Readiness::capture(&spec, changed).is_err());
    let mut changed = raw.clone();
    changed.bridge_generation += 1;
    assert!(Readiness::capture(&spec, changed).is_err());
    let mut changed = raw;
    changed.map_dimensions[0] += 1;
    assert!(Readiness::capture(&spec, changed).is_err());
    Ok(())
}

#[test]
fn finite_lifecycle_uses_durable_boundaries_and_independent_generations() -> Result<()> {
    let (mut clock, _, mut source, c, n) = setup()?;
    assert_eq!(clock.spec().parent().binding().manifest().generation, 7);
    assert_eq!(clock.spec().source().generation(), 29);
    assert_eq!(clock.binding().manifest().generation, 41);
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    let prepared = clock.inspect(&c, &n)?;
    assert!(prepared.pending && !prepared.unresolved);
    assert_eq!(
        prepared.readiness.as_ref().map(Readiness::pending_tiles),
        Some(1)
    );
    let running = commit(&mut clock, &mut source, seal, &c, &n)?;
    assert!(running.unresolved && !running.can_sample_goal());
    assert!(clock.prepared_seal().is_none());
    assert_eq!((source.prepares, source.commits), (1, 1));
    assert!(commit(&mut clock, &mut source, seal, &c, &n).is_err());
    source.finish(1010)?;
    let stopped = clock.reconcile(&mut source, &c, &n)?;
    assert!(stopped.can_sample_goal() && !stopped.pending);
    assert_eq!(stopped.high_tick, 1010);
    assert_eq!(stopped.high_sequence, 4);
    assert!(
        stopped.runs[0]
            .entry
            .native()
            .is_some_and(RunRecord::pause_verified)
    );
    assert_eq!(source.commits, 1);
    assert!(!clock.cached_view().verified);
    assert!(clock.cached_view().readiness.is_none());
    Ok(())
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
    let mut resumed = Clock::new(journal, spec, Some(pin), &c, &n)?;
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
    source.finish(1011)?;
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
    assert!(Clock::new(journal, spec.clone(), None, &c, &n).is_err());
    let journal = open(storage.clone(), &source, &n, RunMode::Control, false)?;
    let mut resumed = Clock::new(journal, spec, Some(pin), &c, &n)?;
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
        let resumed = Clock::new(journal, spec.clone(), Some(pin), &c, &n)?;
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
        assert!(Clock::new(journal, spec.clone(), Some(pin), &c, &n).is_err());
    }
    let journal = open(
        Memory::from_bytes(initial_bytes.clone()),
        &source,
        &n,
        RunMode::Offline,
        false,
    )?;
    assert!(Clock::new(journal, spec.clone(), None, &c, &n).is_err());
    let journal = open(
        Memory::from_bytes(initial_bytes),
        &source,
        &n,
        RunMode::Offline,
        false,
    )?;
    assert!(Clock::new(journal, spec, Some(original), &c, &n).is_ok());
    Ok(())
}

#[test]
fn rejected_future_map_retains_tick_and_cannot_revive_original_expired_authority() -> Result<()> {
    let (mut clock, _, mut source, mut c, mut n) = setup()?;
    let spec = clock.spec().clone();
    for grant in &mut c.grants {
        grant.expires_at_tick = Some(GameTick(1020));
    }
    for grant in &mut n.grants {
        grant.expires_at_tick = Some(GameTick(1020));
    }
    // Make opening authority independently finite, then supply an apparent renewal later.
    clock.authority.canonical = c.grants.clone();
    clock.authority.native = n.grants.clone();
    assert!(
        clock
            .prepare_guarded(&mut source, &request(&spec), &c, &n, |_, _, _| ready(
                &spec, 1030
            ))
            .is_err()
    );
    assert_eq!(clock.high_tick(), 1030);
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
    assert_eq!(clock.cached_view().high_tick, 1030);
    Ok(())
}

#[test]
fn current_and_original_canonical_and_independent_native_scopes_are_all_required() -> Result<()> {
    for variant in 0..8 {
        let (mut clock, _, mut source, mut c, mut n) = setup()?;
        match variant {
            0 => c.grants.retain(|g| g.capability != Capability::Observe),
            1 => c.grants.retain(|g| g.capability != Capability::Plan),
            2 => c
                .grants
                .iter_mut()
                .filter(|g| g.capability == Capability::ControlClock)
                .for_each(|g| g.scope.map_area = Some(clock.spec().parent().binding().scope())),
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
fn fixed_deadline_gap_count_and_original_tick_allowance_are_never_renewed() -> Result<()> {
    let original = fixtures::spec(fixtures::parts())?;
    let spec = changed_policy(&original, Policy::new(1020, 0, 1, 10, 1)?)?;
    let (mut clock, _, mut source, c, n) = with_spec(spec)?;
    let mut excessive = request(clock.spec());
    excessive.game_ticks = 11;
    let target = clock.spec().clone();
    assert!(
        clock
            .prepare_guarded(&mut source, &excessive, &c, &n, |_, p, _| ready(
                &target,
                p.before().tick().ok_or_else(stale)?
            ))
            .is_err()
    );
    assert_eq!(source.prepares, 0);
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    commit(&mut clock, &mut source, seal, &c, &n)?;
    source.finish(1010)?;
    clock.reconcile(&mut source, &c, &n)?;
    assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
    assert_eq!(source.prepares, 1);

    let (mut clock, _, mut source, mut c, mut n) = setup()?;
    clock.authority.game_ticks = 5;
    c.budget.max_game_ticks = 1200;
    n.budget.max_game_ticks = 1200;
    assert_eq!(
        code(prepare(&mut clock, &mut source, &c, &n)),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(source.prepares, 0);
    let (mut clock, _, mut source, c, n) = setup()?;
    source.tick = 1995;
    assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
    assert_eq!(source.prepares, 0);
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
        c.anchor.tick = GameTick(3000);
        n.anchor.tick = GameTick(3000);
        source.tick = 3000;
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
    let operations = storage.operations();
    let journal = open(storage.clone(), &source, &n, RunMode::Offline, false)?;
    let mut recovered = Clock::new(journal, spec, Some(pin), &c, &n)?;
    let current = recovered.inspect(&c, &n)?;
    assert_eq!(current.runs[0].entry.state(), RunState::Tracking);
    assert_eq!(
        current.runs[0].entry.native().map(RunRecord::phase),
        Some(RunPhase::Running)
    );
    assert_eq!(operations, storage.operations());
    assert!(recovered.reconcile(&mut source, &c, &n).is_err());
    assert_eq!(source.queries, 0);
    Ok(())
}

#[test]
fn budget_and_whole_deadline_are_paid_before_any_unpause() -> Result<()> {
    let (mut clock, _, mut source, mut c, mut n) = setup()?;
    c.budget.max_bytes = OPERATION_BYTES - 1;
    n.budget.max_bytes = OPERATION_BYTES - 1;
    assert_eq!(
        code(prepare(&mut clock, &mut source, &c, &n)),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(source.observes, 0);
    c.budget.max_bytes = OPERATION_BYTES;
    n.budget.max_bytes = OPERATION_BYTES;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    c.budget.max_wall_millis = 100;
    n.budget.max_wall_millis = 100;
    let spec = clock.spec().clone();
    assert_eq!(
        code(
            clock.commit_guarded(&mut source, seal, true, &c, &n, |_, p, _| {
                std::thread::sleep(Duration::from_millis(120));
                ready(&spec, p.before().tick().ok_or_else(stale)?)
            })
        ),
        Some(ErrorCode::EffectIndeterminate)
    );
    assert_eq!(source.commits, 0);
    assert!(clock.cached_view().unresolved);
    Ok(())
}

#[test]
fn independent_source_pin_still_requires_original_endpoint_and_software() -> Result<()> {
    let spec = fixtures::spec(fixtures::parts())?;
    let (c, n) = contexts(&spec);
    for different_endpoint in [false, true] {
        let storage = Memory::default();
        let mut source = Source::new(storage.clone());
        if different_endpoint {
            source.endpoint.set_port(5001);
        } else {
            source.manifest.df_version.push_str("-other");
        }
        let journal = open(storage, &source, &n, RunMode::Control, true)?;
        assert_eq!(
            code(Clock::new(journal, spec.clone(), None, &c, &n)),
            Some(ErrorCode::StaleAnchor)
        );
    }
    // The map and run incarnation identifiers deliberately remain independent.
    let (parent, view, records) = fixtures::native_history(fixtures::parts())?;
    let spec_other = MonitorSpec::new(
        parent,
        &view,
        &records,
        SourcePin::new(
            spec.source().endpoint(),
            30,
            "test-df".into(),
            "test-dfhack".into(),
        )?,
        *spec.policy(),
        1000,
    )?;
    assert!(with_spec(spec_other).is_ok());
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
        let mut owner = Clock::new(journal, spec.clone(), Some(pin), &c, &n)?;
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
    let spec = fixtures::spec(fixtures::parts())?;
    let (mut c, mut n) = contexts(&spec);
    for grant in c.grants.iter_mut().chain(n.grants.iter_mut()) {
        grant.expires_at_tick = Some(GameTick(1500));
    }
    let storage = Memory::default();
    let mut source = Source::new(storage.clone());
    let journal = open(storage, &source, &n, RunMode::Control, true)?;
    let mut clock = Clock::new(journal, spec, None, &c, &n)?;
    c.anchor.tick = GameTick(1300);
    n.anchor.tick = GameTick(1300);
    assert_eq!(
        code(prepare(&mut clock, &mut source, &c, &n)),
        Some(ErrorCode::StaleAnchor)
    );
    assert_eq!(clock.high_tick(), 1300);
    assert_eq!(source.prepares, 0);
    assert!(clock.cached_view().runs.is_empty());

    c.anchor.tick = GameTick(1600);
    n.anchor.tick = GameTick(1600);
    assert_eq!(
        code(clock.inspect(&c, &n)),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(clock.high_tick(), 1600);
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
fn precreation_identity_matches_original_header_and_recovers_lost_initial_sync() -> Result<()> {
    let spec = fixtures::spec(fixtures::parts())?;
    let (_, n) = contexts(&spec);
    let storage = Memory::default();
    let source = Source::new(storage.clone());
    let binding = RunBinding::new(source.endpoint, source.manifest.clone())?;
    let intended = binding.initial_journal_id(n.session_id, n.request_id);
    // Frozen independently calculated SHA-256 vector for the existing format.
    assert_eq!(
        intended.to_hex(),
        "a889d7a037dce86baa699296ec27cda9ec91523a1d01c7ff5971fce78d09baf8"
    );
    assert_eq!(storage.operations(), (0, 0, 0));
    let journal = open(storage.clone(), &source, &n, RunMode::Control, true)?;
    assert_eq!(journal.id(), intended);
    assert_eq!(journal.head(), intended);
    assert_eq!(storage.snapshot_bytes().len(), 120);
    assert_ne!(
        binding.initial_journal_id(SessionId::new(2), n.request_id),
        intended
    );
    assert_ne!(
        binding.initial_journal_id(n.session_id, RequestId::new(2)),
        intended
    );
    for changed in [
        RunBinding::new(
            SocketAddr::from(([127, 0, 0, 1], 5001)),
            source.manifest.clone(),
        )?,
        RunBinding::new(
            source.endpoint,
            RunManifest {
                generation: 42,
                ..source.manifest.clone()
            },
        )?,
        RunBinding::new(
            source.endpoint,
            RunManifest {
                df_version: "other-df".into(),
                ..source.manifest.clone()
            },
        )?,
    ] {
        assert_ne!(
            changed.initial_journal_id(n.session_id, n.request_id),
            intended
        );
    }

    let uncertain = Memory::default();
    uncertain.0.borrow_mut().fail_sync = true;
    assert!(open(uncertain.clone(), &source, &n, RunMode::Control, true).is_err());
    assert_eq!(uncertain.snapshot_bytes(), storage.snapshot_bytes());
    let operations = uncertain.operations();
    let mut recovered = open(uncertain.clone(), &source, &n, RunMode::Offline, false)?;
    recovered.verify_recovery(intended, intended, &n)?;
    assert_eq!((recovered.id(), recovered.head()), (intended, intended));
    assert_eq!(uncertain.operations(), operations);
    Ok(())
}
