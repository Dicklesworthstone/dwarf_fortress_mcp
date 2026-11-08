use super::*;
use crate::bounded_run::RunPhase;
use crate::workshop_construction::tests as fixtures;
use dfmcp_core::{CapabilityScope, RequestId};
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
            tick: START,
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

const START: u64 = 12345;
fn target() -> Result<Target> {
    let placed = fixtures::record("placed")?;
    let binding = WorkshopBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        "test-df",
        "test-dfhack",
        placed.plan().before(),
    )?;
    Target::new(
        Digest32::of_bytes(b"original-workshop-journal"),
        binding,
        placed,
        START + 1000,
    )
}
fn contexts(target: &Target) -> (OperationContext, OperationContext) {
    let mut c = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: target.binding().fortress().fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(START),
            state_hash: target.id(),
        },
        budget: WorkBudget {
            max_wall_millis: 60000,
            max_game_ticks: 1200,
            max_entities: 8192,
            max_bytes: 1024 * 1024 * 1024,
            max_output_tokens: 65536,
            max_actions: 1,
        },
        grants: Vec::new(),
        cancellation_requested: false,
    };
    c.grants = [
        Capability::Query,
        Capability::Observe,
        Capability::Plan,
        Capability::ControlClock,
    ]
    .into_iter()
    .map(|capability| CapabilityGrant {
        capability,
        scope: CapabilityScope {
            fortress_id: Some(c.anchor.fortress_id),
            ..CapabilityScope::default()
        },
        max_risk: RiskTier::Guarded,
        expires_at_tick: None,
        remaining_uses: None,
    })
    .collect();
    // These are independently issued native-domain grants, not copied grants.
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
    with_target(target()?)
}
fn with_target(
    target: Target,
) -> Result<(
    Clock<Memory>,
    Memory,
    Source,
    OperationContext,
    OperationContext,
)> {
    let (c, n) = contexts(&target);
    let storage = Memory::default();
    let source = Source::new(storage.clone());
    let journal = open(storage.clone(), &source, &n, RunMode::Control, true)?;
    let clock = Clock::new(journal, target, None, &c, &n)?;
    Ok((clock, storage, source, c, n))
}
fn request(target: &Target) -> RunRequest {
    RunRequest {
        target_id: target.id(),
        game_ticks: 10,
        wall_millis: 500,
    }
}
fn ready(tick: u64) -> Result<WorkshopProgress> {
    let mut raw = fixtures::vector("progress_stage0")?;
    raw[16..24].copy_from_slice(&tick.to_be_bytes());
    WorkshopProgress::decode(&raw)
}
fn prepare(
    clock: &mut Clock<Memory>,
    source: &mut Source,
    c: &OperationContext,
    n: &OperationContext,
) -> Result<Digest32> {
    let target = clock.target().clone();
    let view = clock.prepare_guarded(source, &request(&target), c, n, |edge, p, _| {
        assert_eq!(edge, Edge::Prepare);
        ready(p.before().tick().ok_or_else(stale)?)
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
    clock.commit_guarded(source, seal, true, c, n, |edge, p, _| {
        assert_eq!(edge, Edge::Commit);
        ready(p.before().tick().ok_or_else(stale)?)
    })
}
fn code<T>(result: Result<T>) -> Option<ErrorCode> {
    result.err().map(|error| error.code)
}

#[test]
fn target_binds_exact_resolved_placement_source_and_operator_horizon() -> Result<()> {
    let original = target()?;
    assert_eq!(original.start_tick(), START);
    assert!(
        Target::new(
            Digest32::ZERO,
            original.binding().clone(),
            original.record().clone(),
            original.deadline_tick(),
        )
        .is_err()
    );
    for name in ["prepared", "indeterminate", "refused", "cancelled"] {
        assert!(
            Target::new(
                original.workshop_journal_id(),
                original.binding().clone(),
                fixtures::record(name)?,
                original.deadline_tick()
            )
            .is_err()
        );
    }
    for deadline in [START, START - 1, crate::bounded_run::MAX_NATIVE_TICK + 1] {
        assert!(
            Target::new(
                original.workshop_journal_id(),
                original.binding().clone(),
                original.record().clone(),
                deadline
            )
            .is_err()
        );
    }
    assert!(
        Target::new(
            original.workshop_journal_id(),
            original.binding().recovery_generation(8)?,
            original.record().clone(),
            original.deadline_tick()
        )
        .is_err()
    );
    let changed_journal = Target::new(
        Digest32::of_bytes(b"other-workshop-journal"),
        original.binding().clone(),
        original.record().clone(),
        original.deadline_tick(),
    )?;
    let changed_deadline = Target::new(
        original.workshop_journal_id(),
        original.binding().clone(),
        original.record().clone(),
        original.deadline_tick() + 1,
    )?;
    assert_ne!(original.id(), changed_journal.id());
    assert_ne!(original.id(), changed_deadline.id());
    Ok(())
}

#[test]
fn readiness_requires_original_in_progress_and_never_promotes_completion() -> Result<()> {
    let target = target()?;
    let observed = Readiness::capture(&target, ready(START)?)?;
    assert_eq!(
        (
            observed.stage(),
            observed.max_stage(),
            observed.construct_jobs()
        ),
        (0, 3, 1)
    );
    assert_eq!(observed.digest(), ready(START)?.witness());
    for name in [
        "progress_complete",
        "progress_absent",
        "progress_absent_continuous",
    ] {
        assert!(Readiness::capture(&target, fixtures::progress(name)?).is_err());
    }
    let original = fixtures::vector("progress_stage0")?;
    for (offset, value) in [(15, 8), (27, 2), (60, 43), (74, 13), (102, 7)] {
        let mut raw = original.clone();
        raw[offset] = value;
        assert!(
            WorkshopProgress::decode(&raw)
                .and_then(|progress| Readiness::capture(&target, progress))
                .is_err(),
            "offset {offset}"
        );
    }
    Ok(())
}

#[test]
fn finite_lifecycle_retains_original_target_and_independent_native_generation() -> Result<()> {
    let (mut clock, _, mut source, c, n) = setup()?;
    assert_eq!(clock.target().binding().generation(), 7);
    assert_eq!(clock.binding().manifest().generation, 41);
    let original = clock.target().id();
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    let prepared = clock.inspect(&c, &n)?;
    assert!(prepared.pending && !prepared.unresolved);
    assert!(!prepared.can_sample_goal());
    assert_eq!(prepared.target_id, original);
    assert_eq!(
        prepared.original_plan_digest,
        clock.target().record().plan().digest()
    );
    assert_eq!(prepared.readiness.as_ref().map(Readiness::stage), Some(0));
    let running = commit(&mut clock, &mut source, seal, &c, &n)?;
    assert!(running.unresolved && !running.can_sample_goal());
    assert!(clock.prepared_seal().is_none());
    assert!(commit(&mut clock, &mut source, seal, &c, &n).is_err());
    source.finish(START + 10)?;
    let stopped = clock.reconcile(&mut source, &c, &n)?;
    assert!(stopped.can_sample_goal() && !stopped.pending && stopped.readiness.is_none());
    assert_eq!((stopped.high_tick, stopped.high_sequence), (START + 10, 4));
    assert_eq!((source.prepares, source.commits), (1, 1));
    // A second interval is a new finite review, not repetition of the first.
    let next = prepare(&mut clock, &mut source, &c, &n)?;
    assert_ne!(next, seal);
    assert_eq!(clock.inspect(&c, &n)?.runs.len(), 2);
    assert!(!clock.cached_view().verified);
    assert!(clock.cached_view().readiness.is_none());
    Ok(())
}

#[test]
fn fresh_original_checks_follow_synced_intent_and_dispatch_markers() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let target = clock.target().clone();
    let mut edges = Vec::new();
    let view = clock.prepare_guarded(&mut source, &request(&target), &c, &n, |edge, p, _| {
        edges.push(edge);
        let mut disk = RunJournal::open(storage.clone(), &n, RunMode::Offline, None, false)?;
        assert_eq!(disk.records(&n)?[0].state(), RunState::Intent);
        ready(p.before().tick().ok_or_else(stale)?)
    })?;
    let seal = view
        .active()
        .and_then(|run| run.review_seal)
        .ok_or_else(stale)?;
    clock.commit_guarded(&mut source, seal, true, &c, &n, |edge, p, _| {
        edges.push(edge);
        let mut disk = RunJournal::open(storage.clone(), &n, RunMode::Offline, None, false)?;
        assert_eq!(disk.records(&n)?[0].state(), RunState::DispatchStarted);
        ready(p.before().tick().ok_or_else(stale)?)
    })?;
    assert_eq!(edges, [Edge::Prepare, Edge::Commit]);
    assert_eq!((source.prepares, source.commits), (1, 1));
    Ok(())
}

#[test]
fn completed_or_changed_original_after_durable_marker_refuses_native_control() -> Result<()> {
    for at_commit in [false, true] {
        for name in ["progress_complete", "progress_absent_continuous"] {
            let (mut clock, _, mut source, c, n) = setup()?;
            let result = if at_commit {
                let seal = prepare(&mut clock, &mut source, &c, &n)?;
                clock.commit_guarded(&mut source, seal, true, &c, &n, |_, _, _| {
                    fixtures::progress(name)
                })
            } else {
                let request = request(clock.target());
                clock.prepare_guarded(&mut source, &request, &c, &n, |_, _, _| {
                    fixtures::progress(name)
                })
            };
            assert!(result.is_err());
            assert_eq!(source.commits, 0);
            assert_eq!(source.prepares, usize::from(at_commit));
            assert!(clock.prepared_seal().is_none());
            assert!(clock.cached_view().pending);
            assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
        }
    }
    Ok(())
}

#[test]
fn wrong_seal_or_withdrawn_confirmation_consumes_the_only_review() -> Result<()> {
    for confirmed in [false, true] {
        let (mut clock, _, mut source, c, n) = setup()?;
        let seal = prepare(&mut clock, &mut source, &c, &n)?;
        assert!(
            clock
                .commit_guarded(&mut source, Digest32::ZERO, confirmed, &c, &n, |_, _, _| {
                    ready(START)
                })
                .is_err()
        );
        assert!(clock.prepared_seal().is_none());
        assert!(clock.inspect(&c, &n)?.readiness.is_none());
        assert!(commit(&mut clock, &mut source, seal, &c, &n).is_err());
        assert_eq!(source.commits, 0);
        assert!(!clock.cancel(None, &c, &n)?.pending);
        assert_eq!(source.cancels, 0);
    }
    Ok(())
}

#[test]
fn uncertain_dispatch_or_absent_recovery_never_retries_unpause() -> Result<()> {
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
    source.finish(START + 11)?;
    assert!(!clock.reconcile(&mut source, &c, &n)?.pending);
    assert_eq!(source.commits, 1);
    Ok(())
}

#[test]
fn recovery_requires_external_pin_and_cannot_restore_control_or_change_target() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    let pin = clock.recovery();
    let target = clock.target().clone();
    drop(clock);
    for mode in [RunMode::Control, RunMode::Recover, RunMode::Offline] {
        let journal = open(storage.clone(), &source, &n, mode, false)?;
        assert!(Clock::new(journal, target.clone(), None, &c, &n).is_err());
        let journal = open(storage.clone(), &source, &n, mode, false)?;
        let recovered = Clock::new(journal, target.clone(), Some(pin), &c, &n);
        if mode == RunMode::Control {
            assert!(recovered.is_err());
        } else {
            let mut owner = recovered?;
            assert!(owner.inspect(&c, &n)?.runs[0].review_seal.is_none());
            assert!(commit(&mut owner, &mut source, seal, &c, &n).is_err());
            assert!(prepare(&mut owner, &mut source, &c, &n).is_err());
            assert!(owner.cancel(None, &c, &n).is_err());
        }
    }
    for different in [
        Target::new(
            target.workshop_journal_id(),
            target.binding().clone(),
            target.record().clone(),
            target.deadline_tick() + 1,
        )?,
        Target::new(
            Digest32::of_bytes(b"other-original"),
            target.binding().clone(),
            target.record().clone(),
            target.deadline_tick(),
        )?,
    ] {
        let journal = open(storage.clone(), &source, &n, RunMode::Offline, false)?;
        assert!(Clock::new(journal, different, Some(pin), &c, &n).is_err());
    }
    assert_eq!((source.commits, source.cancels), (0, 0));
    Ok(())
}

#[test]
fn external_head_rejects_rollback_but_accepts_complete_descendants() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let original_pin = clock.recovery();
    let original_bytes = storage.snapshot_bytes();
    let target = clock.target().clone();
    prepare(&mut clock, &mut source, &c, &n)?;
    let acknowledged = clock.recovery();
    for pin in [original_pin, acknowledged] {
        let journal = open(
            Memory::from_bytes(storage.snapshot_bytes()),
            &source,
            &n,
            RunMode::Offline,
            false,
        )?;
        assert_eq!(
            Clock::new(journal, target.clone(), Some(pin), &c, &n)?.recovery(),
            acknowledged
        );
    }
    for pin in [
        acknowledged,
        Recovery {
            journal_id: original_pin.journal_id,
            head: Digest32::ZERO,
        },
        Recovery {
            journal_id: Digest32::ZERO,
            head: original_pin.head,
        },
    ] {
        let journal = open(
            Memory::from_bytes(original_bytes.clone()),
            &source,
            &n,
            RunMode::Offline,
            false,
        )?;
        assert!(Clock::new(journal, target.clone(), Some(pin), &c, &n).is_err());
    }
    Ok(())
}

#[test]
fn rejected_future_progress_retains_floor_and_opening_grant_expiry() -> Result<()> {
    let (mut clock, _, mut source, mut c, mut n) = setup()?;
    for grant in c.grants.iter_mut().chain(n.grants.iter_mut()) {
        grant.expires_at_tick = Some(GameTick(START + 20));
    }
    clock.authority.canonical = c.grants.clone();
    clock.authority.native = n.grants.clone();
    let request = request(clock.target());
    assert!(
        clock
            .prepare_guarded(&mut source, &request, &c, &n, |_, _, _| ready(START + 30))
            .is_err()
    );
    assert_eq!(clock.high_tick(), START + 30);
    assert_eq!(source.prepares, 0);
    for grant in c.grants.iter_mut().chain(n.grants.iter_mut()) {
        grant.expires_at_tick = None;
    }
    c.anchor.tick = GameTick(0);
    n.anchor.tick = GameTick(0);
    assert_eq!(
        code(clock.inspect(&c, &n)),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(clock.cached_view().high_tick, START + 30);
    Ok(())
}

#[test]
fn both_current_and_opening_native_and_named_authorities_are_required() -> Result<()> {
    for variant in 0..9 {
        let (mut clock, _, mut source, mut c, mut n) = setup()?;
        match variant {
            0 => c.grants.retain(|g| g.capability != Capability::Observe),
            1 => c.grants.retain(|g| g.capability != Capability::Plan),
            2 => c
                .grants
                .retain(|g| g.capability != Capability::ControlClock),
            3 => n.grants.retain(|g| g.capability != Capability::Plan),
            4 => n.anchor.fortress_id = c.anchor.fortress_id,
            5 => n.request_id = RequestId::new(900),
            6 => clock
                .authority
                .native
                .retain(|g| g.capability != Capability::ControlClock),
            7 => clock
                .authority
                .canonical
                .retain(|g| g.capability != Capability::Plan),
            _ => c.session_id = SessionId::new(900),
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
fn immutable_deadline_and_original_game_budget_cannot_be_extended() -> Result<()> {
    let original = target()?;
    let finite = Target::new(
        original.workshop_journal_id(),
        original.binding().clone(),
        original.record().clone(),
        START + 10,
    )?;
    let (mut clock, _, mut source, c, n) = with_target(finite)?;
    let mut excessive = request(clock.target());
    excessive.game_ticks = 11;
    assert!(
        clock
            .prepare_guarded(&mut source, &excessive, &c, &n, |_, p, _| ready(
                p.before().tick().ok_or_else(stale)?
            ))
            .is_err()
    );
    assert_eq!(source.prepares, 0);
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    commit(&mut clock, &mut source, seal, &c, &n)?;
    source.finish(START + 10)?;
    clock.reconcile(&mut source, &c, &n)?;
    assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
    assert_eq!(source.prepares, 1);
    let (mut clock, _, mut source, c, n) = setup()?;
    clock.authority.game_ticks = 5;
    assert_eq!(
        code(prepare(&mut clock, &mut source, &c, &n)),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(source.prepares, 0);
    Ok(())
}

#[test]
fn safety_stop_survives_expired_goal_and_observe_or_plan_revocation() -> Result<()> {
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
        c.anchor.tick = GameTick(START + 2000);
        n.anchor.tick = c.anchor.tick;
        source.tick = START + 2000;
        let view = clock.cancel(if running { Some(&mut source) } else { None }, &c, &n)?;
        assert!(!view.pending);
        assert_eq!(source.cancels, usize::from(running));
        assert!(clock.prepared_seal().is_none());
    }
    Ok(())
}

#[test]
fn corruption_during_fresh_guard_blocks_both_native_boundaries() -> Result<()> {
    for at_commit in [false, true] {
        let (mut clock, storage, mut source, c, n) = setup()?;
        if at_commit {
            let seal = prepare(&mut clock, &mut source, &c, &n)?;
            assert!(
                clock
                    .commit_guarded(&mut source, seal, true, &c, &n, |_, p, _| {
                        storage.damage();
                        ready(p.before().tick().ok_or_else(stale)?)
                    })
                    .is_err()
            );
        } else {
            let request = request(clock.target());
            assert!(
                clock
                    .prepare_guarded(&mut source, &request, &c, &n, |_, p, _| {
                        storage.damage();
                        ready(p.before().tick().ok_or_else(stale)?)
                    })
                    .is_err()
            );
        }
        assert_eq!(source.commits, 0);
        assert!(clock.cached_view().unresolved);
        assert!(clock.prepared_seal().is_none());
        assert!(clock.inspect(&c, &n).is_err());
    }
    Ok(())
}

#[test]
fn rejected_native_source_floor_and_full_operation_budget_are_retained() -> Result<()> {
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
    source.tick = START + 100;
    source.sequence = 10;
    source.paused = false;
    assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
    assert_eq!(
        (clock.high_tick(), clock.high_sequence()),
        (START + 100, 10)
    );
    source.tick = START;
    source.sequence = 3;
    source.paused = true;
    assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
    assert_eq!(source.prepares, 0);
    Ok(())
}

#[test]
fn independent_run_pin_still_requires_workshop_endpoint_and_software() -> Result<()> {
    let target = target()?;
    let (c, n) = contexts(&target);
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
            code(Clock::new(journal, target.clone(), None, &c, &n)),
            Some(ErrorCode::StaleAnchor)
        );
    }
    Ok(())
}

#[test]
fn retained_terminal_unknown_blocks_new_run_and_never_claims_pause() -> Result<()> {
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
            .is_some_and(|r| !r.pause_verified())
    );
    assert!(prepare(&mut clock, &mut source, &c, &n).is_err());
    assert_eq!(source.commits, 1);
    Ok(())
}

#[test]
fn lost_receipt_sync_preserves_floor_and_query_only_recovery() -> Result<()> {
    let (mut clock, storage, mut source, c, n) = setup()?;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    let pin = clock.recovery();
    let target = clock.target().clone();
    source.fail_receipt_sync = true;
    assert_eq!(
        code(commit(&mut clock, &mut source, seal, &c, &n)),
        Some(ErrorCode::EffectIndeterminate)
    );
    assert_eq!(clock.high_sequence(), 4);
    assert!(clock.cached_view().unresolved);
    drop(clock);
    let mut query_c = c.clone();
    let mut query_n = n.clone();
    query_c.grants.retain(|g| g.capability == Capability::Query);
    query_n.grants.retain(|g| g.capability == Capability::Query);
    let journal = open(storage, &source, &query_n, RunMode::Recover, false)?;
    let mut recovered = Clock::new(journal, target, Some(pin), &query_c, &query_n)?;
    assert_eq!(recovered.inspect(&query_c, &query_n)?.high_sequence, 4);
    source.finish(START + 10)?;
    let stopped = recovered.reconcile(&mut source, &query_c, &query_n)?;
    assert!(stopped.verified && !stopped.pending);
    assert!(prepare(&mut recovered, &mut source, &c, &n).is_err());
    assert_eq!(source.commits, 1);
    Ok(())
}

#[test]
fn whole_operation_deadline_expires_before_native_dispatch() -> Result<()> {
    let (mut clock, _, mut source, mut c, mut n) = setup()?;
    let seal = prepare(&mut clock, &mut source, &c, &n)?;
    c.budget.max_wall_millis = 100;
    n.budget.max_wall_millis = 100;
    assert_eq!(
        code(
            clock.commit_guarded(&mut source, seal, true, &c, &n, |_, p, _| {
                std::thread::sleep(Duration::from_millis(120));
                ready(p.before().tick().ok_or_else(stale)?)
            })
        ),
        Some(ErrorCode::EffectIndeterminate)
    );
    assert_eq!(source.commits, 0);
    assert!(clock.cached_view().unresolved);
    assert!(clock.prepared_seal().is_none());
    Ok(())
}

#[test]
fn full_interval_history_is_retained_and_the_sixty_fifth_is_refused() -> Result<()> {
    let (mut clock, _, mut source, c, n) = setup()?;
    for interval in 0..MAX_RUNS {
        source.tick = START + interval as u64;
        source.sequence = 3 + interval as u64;
        prepare(&mut clock, &mut source, &c, &n)?;
        assert!(!clock.cancel(None, &c, &n)?.pending);
    }
    let observes = source.observes;
    assert_eq!(
        code(prepare(&mut clock, &mut source, &c, &n)),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(clock.inspect(&c, &n)?.runs.len(), MAX_RUNS);
    assert_eq!(source.observes, observes);
    assert_eq!(
        (source.prepares, source.commits, source.cancels),
        (MAX_RUNS, 0, 0)
    );
    Ok(())
}
