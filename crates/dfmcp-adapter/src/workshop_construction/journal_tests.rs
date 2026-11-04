//! Actual coordinator transitions over in-memory custody and a scripted source
//! that replays the actual C++ handler vectors. Component evidence only.
use super::*;
use crate::workshop_construction::rpc::WorkshopManifest;
use crate::workshop_construction::tests::{capture, plan, progress, record};
use crate::workshop_construction::{WorkshopCapture, WorkshopPreparation};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Read, Seek, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn endpoint() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 5000))
}
fn binding() -> Result<WorkshopBinding> {
    WorkshopBinding::new(endpoint(), "test-df", "test-dfhack", &capture()?)
}
pub(crate) fn context(session: u128) -> Result<OperationContext> {
    let before = capture()?;
    let fortress_id = before.fortress_id();
    Ok(OperationContext {
        session_id: SessionId::new(session),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(before.tick()),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 512 * 1024 * 1024,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: [
            Capability::Query,
            Capability::Observe,
            Capability::Plan,
            Capability::Construct,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            max_risk: RiskTier::Guarded,
            scope: CapabilityScope {
                fortress_id: Some(fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    })
}
fn query_only(session: u128) -> Result<OperationContext> {
    let mut c = context(session)?;
    c.grants.retain(|g| g.capability == Capability::Query);
    Ok(c)
}

#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
    fail_write_after: Option<usize>,
    writes: usize,
}
#[derive(Clone, Default)]
struct Memory {
    disk: Rc<RefCell<Disk>>,
    position: usize,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let disk = self.disk.borrow();
        let start = self.position.min(disk.bytes.len());
        let count = out.len().min(disk.bytes.len() - start);
        out[..count].copy_from_slice(&disk.bytes[start..start + count]);
        self.position += count;
        Ok(count)
    }
}
impl Seek for Memory {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        let next = match offset {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.disk.borrow().bytes.len() as i128 + i128::from(n),
            SeekFrom::Current(n) => self.position as i128 + i128::from(n),
        };
        self.position = usize::try_from(next).map_err(|_| io::Error::other("invalid seek"))?;
        Ok(self.position as u64)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        disk.writes += 1;
        if disk.fail_write_after.is_some_and(|n| disk.writes > n) {
            // A torn append: some bytes reach storage, publication fails.
            disk.bytes.extend_from_slice(&bytes[..bytes.len().min(5)]);
            return Err(io::Error::other("injected torn write"));
        }
        if self.position != disk.bytes.len() {
            return Err(io::Error::other("append failure"));
        }
        disk.bytes.extend_from_slice(bytes);
        self.position += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair forbidden"))
    }
}

#[derive(Default)]
struct Calls {
    observed: usize,
    prepared: usize,
    committed: usize,
    queried: usize,
    cancelled: usize,
    progressed: usize,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Commit {
    Placed,
    Indeterminate,
    LostReply,
}
struct Native {
    manifest: WorkshopManifest,
    calls: Rc<RefCell<Calls>>,
    observation: WorkshopCapture,
    commit: Commit,
    query: Option<WorkshopRecord>,
    progress: Vec<WorkshopProgress>,
    unresolved: bool,
}
impl Native {
    fn new() -> Result<Self> {
        let before = capture()?;
        Ok(Self {
            manifest: WorkshopManifest {
                endpoint: endpoint(),
                fortress: before.fortress().clone(),
                generation: before.generation(),
                df_version: "test-df".to_owned(),
                dfhack_version: "test-dfhack".to_owned(),
            },
            calls: Rc::default(),
            observation: before,
            commit: Commit::Placed,
            query: None,
            progress: Vec::new(),
            unresolved: false,
        })
    }
}
impl WorkshopSource for Native {
    fn manifest(&self) -> &WorkshopManifest {
        &self.manifest
    }
    fn native_summary(&self) -> super::super::WorkshopNativeSummary {
        super::super::WorkshopNativeSummary {
            unresolved: self.unresolved,
            retained_records: u16::from(self.unresolved),
        }
    }
    fn fence(&mut self) {}
    fn observe(
        &mut self,
        _: WorkshopSelection,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopCapture> {
        self.calls.borrow_mut().observed += 1;
        Ok(self.observation.clone())
    }
    fn prepare(
        &mut self,
        _: &WorkshopPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopPreparation> {
        self.calls.borrow_mut().prepared += 1;
        WorkshopPreparation::new(record("prepared")?, false)
    }
    fn commit(
        &mut self,
        dispatch: WorkshopDispatch<'_>,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopRecord> {
        self.calls.borrow_mut().committed += 1;
        assert_ne!(dispatch.journal_head(), Digest32::ZERO);
        match self.commit {
            Commit::Placed => record("placed"),
            Commit::Indeterminate => {
                self.unresolved = true;
                record("indeterminate")
            }
            Commit::LostReply => {
                self.unresolved = true;
                Err(error(
                    ErrorCode::AdapterUnavailable,
                    "reply lost after send",
                ))
            }
        }
    }
    fn query(
        &mut self,
        _: &WorkshopPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<Option<WorkshopRecord>> {
        self.calls.borrow_mut().queried += 1;
        Ok(self.query.clone())
    }
    fn cancel(
        &mut self,
        _: &WorkshopPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopRecord> {
        self.calls.borrow_mut().cancelled += 1;
        record("cancelled")
    }
    fn progress(
        &mut self,
        building: u32,
        item: u32,
        _: &OperationContext,
        _: Duration,
    ) -> Result<WorkshopProgress> {
        self.calls.borrow_mut().progressed += 1;
        assert_eq!((building, item), (70, 42));
        if self.progress.is_empty() {
            return Err(error(ErrorCode::AdapterUnavailable, "progress unavailable"));
        }
        Ok(self.progress.remove(0))
    }
}

#[derive(Default)]
struct Guard {
    checks: Vec<WorkshopStage>,
    refuse: Option<WorkshopStage>,
}
impl WorkshopGuard for Guard {
    fn acknowledge(&mut self, _: WorkshopPin, _: &OperationContext) -> Result<()> {
        Ok(())
    }
    fn check(
        &mut self,
        stage: WorkshopStage,
        _: &WorkshopBinding,
        selection: WorkshopSelection,
        item: Option<[u32; 3]>,
        _: &OperationContext,
    ) -> Result<()> {
        assert_eq!(selection.center(), [15, 15, 2]);
        assert_eq!(item, Some([10, 11, 2]));
        self.checks.push(stage);
        if self.refuse == Some(stage) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "lease expired or scope revoked",
            ));
        }
        Ok(())
    }
}

fn fresh(memory: &Memory, session: u128) -> Result<WorkshopJournal<Memory>> {
    WorkshopJournal::open(
        memory.clone(),
        &context(session)?,
        WorkshopMode::Control,
        Some(binding()?),
        Some([7; 32]),
        None,
    )
}
fn reopen(memory: &Memory, mode: WorkshopMode, session: u128) -> Result<WorkshopJournal<Memory>> {
    WorkshopJournal::open(memory.clone(), &context(session)?, mode, None, None, None)
}

#[test]
fn positive_construction_then_fresh_original_completion() -> Result<()> {
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    let mut native = Native::new()?;
    let mut guard = Guard::default();
    let plan = plan()?;
    let entry = journal.prepare(&mut native, &plan, &context(1)?, &mut guard)?;
    assert_eq!(entry.state(), WorkshopState::Prepared);
    assert!(journal.has_permit("workshop-001"));
    let entry = journal.commit(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    )?;
    assert_eq!(entry.state(), WorkshopState::Terminal);
    assert!(entry.placed() && entry.dispatch_started() && !entry.unresolved());
    assert!(guard.checks.contains(&WorkshopStage::Commit));
    assert_eq!(native.calls.borrow().committed, 1);
    // Stage zero is registration; the goal is still in progress.
    native.progress = vec![progress("progress_stage0")?, progress("progress_complete")?];
    let staged = journal.sample(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    )?;
    assert!(matches!(
        staged.goal(),
        Some(WorkshopGoal::InProgress { stage: 0, .. })
    ));
    let done = journal.sample(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    )?;
    assert_eq!(done.goal(), Some(WorkshopGoal::Complete));
    assert_eq!(done.samples(), 2);
    // A proved goal is immutable: no further native read is made.
    let again = journal.sample(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    )?;
    assert_eq!(again, done);
    assert_eq!(native.calls.borrow().progressed, 2);
    // Duplicate commit returns history without a second writer call.
    journal.commit(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    )?;
    assert_eq!(native.calls.borrow().committed, 1);
    drop(journal);
    // Restart: offline reopen preserves the original goal and its proof.
    let mut offline = reopen(&memory, WorkshopMode::Offline, 2)?;
    let view = offline.inventory(&context(2)?)?;
    let entry = view.entry("workshop-001").ok_or_else(corrupt)?;
    assert_eq!(entry.goal(), Some(WorkshopGoal::Complete));
    assert_eq!(
        entry.native().map(WorkshopRecord::canonical_bytes),
        Some(record("placed")?.canonical_bytes())
    );
    assert!(
        offline
            .sample(
                &mut native,
                "workshop-001",
                plan.digest(),
                &context(2)?,
                &mut guard
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn lost_reply_never_redispatches_and_absence_stays_unresolved() -> Result<()> {
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    let mut native = Native::new()?;
    native.commit = Commit::LostReply;
    let mut guard = Guard::default();
    let plan = plan()?;
    journal.prepare(&mut native, &plan, &context(1)?, &mut guard)?;
    let lost = journal.commit(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    );
    assert_eq!(
        lost.map(|_| ()).map_err(|e| e.code),
        Err(ErrorCode::EffectIndeterminate)
    );
    // The second commit is refused locally; the writer is never reached again.
    let again = journal.commit(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    );
    assert_eq!(
        again.map(|_| ()).map_err(|e| e.code),
        Err(ErrorCode::EffectIndeterminate)
    );
    assert_eq!(native.calls.borrow().committed, 1);
    let view = journal.inventory(&context(1)?)?;
    let entry = view.entry("workshop-001").ok_or_else(corrupt)?;
    assert_eq!(entry.state(), WorkshopState::DispatchStarted);
    assert!(entry.unresolved());
    // A new key cannot turn uncertainty green.
    let other = WorkshopPlan::new("workshop-002", capture()?)?;
    let refused = journal.prepare(&mut native, &other, &context(1)?, &mut guard);
    assert_eq!(
        refused.map(|_| ()).map_err(|e| e.code),
        Err(ErrorCode::EffectIndeterminate)
    );
    assert_eq!(native.calls.borrow().prepared, 1);
    // Native absence after restart is not nonapplication.
    native.query = None;
    let absent = journal.recover(
        &mut native,
        "workshop-001",
        plan.digest(),
        false,
        &query_only(1)?,
        &mut guard,
    );
    assert_eq!(
        absent.map(|_| ()).map_err(|e| e.code),
        Err(ErrorCode::EffectIndeterminate)
    );
    // Cancellation after dispatch only queries; it never retires or undoes.
    native.query = Some(record("indeterminate")?);
    let uncertain = journal.recover(
        &mut native,
        "workshop-001",
        plan.digest(),
        true,
        &query_only(1)?,
        &mut guard,
    )?;
    assert_eq!(uncertain.state(), WorkshopState::Terminal);
    assert!(uncertain.unresolved() && !uncertain.cancel_requested());
    assert_eq!(native.calls.borrow().cancelled, 0);
    // Only the exact original Placed receipt discharges it, read-only.
    native.query = Some(record("placed")?);
    let healed = journal.recover(
        &mut native,
        "workshop-001",
        plan.digest(),
        false,
        &query_only(1)?,
        &mut guard,
    )?;
    assert!(healed.placed() && !healed.unresolved());
    assert_eq!(native.calls.borrow().committed, 1);
    // Settled history returns locally; a later contradictory native reply is
    // never even requested, let alone adopted.
    native.query = Some(record("refused")?);
    let queries = native.calls.borrow().queried;
    let settled = journal.recover(
        &mut native,
        "workshop-001",
        plan.digest(),
        false,
        &query_only(1)?,
        &mut guard,
    )?;
    assert!(settled.placed() && native.calls.borrow().queried == queries);
    Ok(())
}

#[test]
fn restart_restores_evidence_but_never_dispatch_permission() -> Result<()> {
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    let mut native = Native::new()?;
    let mut guard = Guard::default();
    let plan = plan()?;
    journal.prepare(&mut native, &plan, &context(1)?, &mut guard)?;
    let acknowledged = journal.pin();
    drop(journal);
    let mut reopened = WorkshopJournal::open(
        memory.clone(),
        &context(2)?,
        WorkshopMode::Control,
        Some(binding()?),
        None,
        Some(acknowledged),
    )?;
    assert!(!reopened.has_permit("workshop-001"));
    let refused = reopened.commit(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(2)?,
        &mut guard,
    );
    assert_eq!(
        refused.map(|_| ()).map_err(|e| e.code),
        Err(ErrorCode::EffectIndeterminate)
    );
    assert_eq!(native.calls.borrow().committed, 0);
    // Replaying preparation with the same key returns history, not permission.
    let replay = reopened.prepare(&mut native, &plan, &context(2)?, &mut guard)?;
    assert_eq!(replay.state(), WorkshopState::Prepared);
    assert!(!reopened.has_permit("workshop-001") && native.calls.borrow().prepared == 1);
    // Retiring the unattempted preparation needs only Query authority.
    let cancelled = reopened.recover(
        &mut native,
        "workshop-001",
        plan.digest(),
        true,
        &query_only(2)?,
        &mut guard,
    )?;
    assert_eq!(
        cancelled.native().map(WorkshopRecord::phase),
        Some(WorkshopPhase::Cancelled)
    );
    // The same key never names another plan.
    let mut other = capture()?.canonical_bytes().to_vec();
    other[23] = 1; // a different native sequence
    let changed = WorkshopPlan::new("workshop-001", WorkshopCapture::decode(&other)?)?;
    let conflict = reopened.prepare(&mut native, &changed, &context(2)?, &mut guard);
    assert_eq!(
        conflict.map(|_| ()).map_err(|e| e.code),
        Err(ErrorCode::Conflict)
    );
    Ok(())
}

#[test]
fn stale_source_and_guard_refusals_happen_before_the_writer() -> Result<()> {
    // Stale observation at commit: no dispatch frame, no writer.
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    let mut native = Native::new()?;
    let mut guard = Guard::default();
    let plan = plan()?;
    journal.prepare(&mut native, &plan, &context(1)?, &mut guard)?;
    let mut moved = capture()?.canonical_bytes().to_vec();
    moved[16 + 7] ^= 1; // a later native intervention sequence
    native.observation = WorkshopCapture::decode(&moved)?;
    assert!(
        journal
            .commit(
                &mut native,
                "workshop-001",
                plan.digest(),
                &context(1)?,
                &mut guard
            )
            .is_err()
    );
    let entry = journal.get("workshop-001", plan.digest(), &context(1)?)?;
    assert_eq!(entry.state(), WorkshopState::Prepared);
    assert!(!entry.dispatch_started() && native.calls.borrow().committed == 0);
    // Guard refusal after the synchronized dispatch marker: no writer call.
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    let mut native = Native::new()?;
    let mut guard = Guard::default();
    journal.prepare(&mut native, &plan, &context(1)?, &mut guard)?;
    guard.refuse = Some(WorkshopStage::Commit);
    let refused = journal.commit(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    );
    assert_eq!(
        refused.map(|_| ()).map_err(|e| e.code),
        Err(ErrorCode::EffectIndeterminate)
    );
    let entry = journal.get("workshop-001", plan.digest(), &context(1)?)?;
    assert!(entry.dispatch_started() && entry.unresolved());
    assert_eq!(native.calls.borrow().committed, 0);
    // A changed native generation is never the original effect source.
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    let mut native = Native::new()?;
    let mut guard = Guard::default();
    journal.prepare(&mut native, &plan, &context(1)?, &mut guard)?;
    native.manifest.generation += 1;
    assert!(
        journal
            .commit(
                &mut native,
                "workshop-001",
                plan.digest(),
                &context(1)?,
                &mut guard
            )
            .is_err()
    );
    assert_eq!(native.calls.borrow().committed, 0);
    // Without Construct authority there is no preparation at all.
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    let mut native = Native::new()?;
    let refused = journal.prepare(&mut native, &plan, &query_only(1)?, &mut Guard::default());
    assert_eq!(
        refused.map(|_| ()).map_err(|e| e.code),
        Err(ErrorCode::CapabilityDenied)
    );
    assert_eq!(native.calls.borrow().observed, 0);
    Ok(())
}

#[test]
fn torn_publication_fences_and_corrupt_history_is_never_repaired() -> Result<()> {
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    let mut native = Native::new()?;
    let mut guard = Guard::default();
    let plan = plan()?;
    journal.prepare(&mut native, &plan, &context(1)?, &mut guard)?;
    // The dispatch frame tears: the writer must not run.
    let acknowledged = journal.pin();
    let writes = memory.disk.borrow().writes;
    memory.disk.borrow_mut().fail_write_after = Some(writes);
    let torn = journal.commit(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    );
    assert!(torn.is_err());
    assert_eq!(native.calls.borrow().committed, 0);
    assert!(journal.is_fenced());
    drop(journal);
    let reopened = WorkshopJournal::open(
        memory.clone(),
        &context(2)?,
        WorkshopMode::Recover,
        None,
        None,
        Some(acknowledged),
    );
    assert_eq!(
        reopened.map(|_| ()).map_err(|e| e.code),
        Err(ErrorCode::CorruptLedger)
    );
    // Flipped history bytes are refused, not truncated away.
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    journal.prepare(
        &mut Native::new()?,
        &plan,
        &context(1)?,
        &mut Guard::default(),
    )?;
    drop(journal);
    let n = memory.disk.borrow().bytes.len();
    memory.disk.borrow_mut().bytes[n - 50] ^= 1;
    let reopened = reopen(&memory, WorkshopMode::Offline, 2);
    assert_eq!(
        reopened.map(|_| ()).map_err(|e| e.code),
        Err(ErrorCode::CorruptLedger)
    );
    // Another session cannot read this owner's custody.
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    assert!(journal.inventory(&context(9)?).is_err());
    Ok(())
}

#[test]
fn completion_samples_preserve_unknown_and_absence() -> Result<()> {
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    let mut native = Native::new()?;
    let mut guard = Guard::default();
    let plan = plan()?;
    // No placed receipt: sampling is refused without a native read.
    journal.prepare(&mut native, &plan, &context(1)?, &mut guard)?;
    assert!(
        journal
            .sample(
                &mut native,
                "workshop-001",
                plan.digest(),
                &context(1)?,
                &mut guard
            )
            .is_err()
    );
    assert_eq!(native.calls.borrow().progressed, 0);
    journal.commit(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    )?;
    // A same-ID complete-looking building under another native generation is Unknown.
    let mut raw = progress("progress_complete")?.canonical_bytes().to_vec();
    raw[14] ^= 1; // a later plugin incarnation (generation + 256)
    native.manifest.generation = u64::from_be_bytes(raw[8..16].try_into().map_err(|_| corrupt())?);
    native.progress = vec![
        WorkshopProgress::decode(&raw)?,
        progress("progress_absent")?,
    ];
    let restarted = journal.sample(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    )?;
    assert_eq!(
        restarted.goal(),
        Some(WorkshopGoal::Unknown(
            super::super::WorkshopUnknown::NativeGenerationChanged
        ))
    );
    assert!(!restarted.goal_complete());
    // Returning to the original generation cannot erase the continuity break.
    native.manifest.generation = capture()?.generation();
    assert!(
        journal
            .sample(
                &mut native,
                "workshop-001",
                plan.digest(),
                &context(1)?,
                &mut guard
            )
            .is_err()
    );
    assert_eq!(
        journal
            .get("workshop-001", plan.digest(), &context(1)?)?
            .goal(),
        restarted.goal()
    );
    drop(journal);
    // Independently, an unbroken source retains observed absence as absence.
    let memory = Memory::default();
    let mut journal = fresh(&memory, 1)?;
    let mut native = Native::new()?;
    journal.prepare(&mut native, &plan, &context(1)?, &mut guard)?;
    journal.commit(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    )?;
    native.progress = vec![progress("progress_absent_continuous")?];
    let absent = journal.sample(
        &mut native,
        "workshop-001",
        plan.digest(),
        &context(1)?,
        &mut guard,
    )?;
    assert_eq!(absent.goal(), Some(WorkshopGoal::Absent));
    // A failed native read appends nothing and invents nothing.
    let before = journal.inventory(&context(1)?)?.frames;
    assert!(
        journal
            .sample(
                &mut native,
                "workshop-001",
                plan.digest(),
                &context(1)?,
                &mut guard
            )
            .is_err()
    );
    assert_eq!(journal.inventory(&context(1)?)?.frames, before);
    // Restart preserves the latest observed absence for the original goal.
    drop(journal);
    let mut offline = reopen(&memory, WorkshopMode::Offline, 3)?;
    let entry = offline.get("workshop-001", plan.digest(), &context(3)?)?;
    assert_eq!(
        (entry.samples(), entry.goal()),
        (1, Some(WorkshopGoal::Absent))
    );
    Ok(())
}

fn sampled(name: &str, generation: u64, tick: u64) -> Result<WorkshopProgress> {
    let mut raw = progress(name)?.canonical_bytes().to_vec();
    raw[8..16].copy_from_slice(&generation.to_be_bytes());
    raw[16..24].copy_from_slice(&tick.to_be_bytes());
    WorkshopProgress::decode(&raw)
}

#[test]
fn full_history_generation_and_clock_regressions_never_complete() -> Result<()> {
    let p = plan()?;
    let g = p.before().generation();
    let t = p.before().tick();
    for generation_break in [false, true] {
        let memory = Memory::default();
        let mut j = fresh(&memory, 1)?;
        let mut n = Native::new()?;
        let mut guard = Guard::default();
        j.prepare(&mut n, &p, &context(1)?, &mut guard)?;
        j.commit(&mut n, p.key(), p.digest(), &context(1)?, &mut guard)?;
        n.progress = vec![sampled("progress_stage0", g, t + 200)?];
        j.sample(&mut n, p.key(), p.digest(), &context(1)?, &mut guard)?;
        if generation_break {
            n.manifest.generation = g + 1;
            n.progress = vec![sampled("progress_stage0", g + 1, t + 100)?];
            let e = j.sample(&mut n, p.key(), p.digest(), &context(1)?, &mut guard)?;
            assert_eq!(
                e.goal(),
                Some(WorkshopGoal::Unknown(
                    super::super::WorkshopUnknown::NativeGenerationChanged
                ))
            );
            assert_eq!(e.high_tick(), t + 200);
        }
        let acknowledged = j.pin();
        drop(j);
        let mut j = WorkshopJournal::open(
            memory.clone(),
            &context(2)?,
            WorkshopMode::Recover,
            None,
            None,
            Some(acknowledged),
        )?;
        n.manifest.generation = g;
        n.progress = vec![sampled("progress_complete", g, t + 150)?];
        let frames = j.inventory(&context(2)?)?.frames;
        assert!(
            j.sample(&mut n, p.key(), p.digest(), &context(2)?, &mut guard)
                .is_err()
        );
        assert_eq!(j.inventory(&context(2)?)?.frames, frames);
        assert!(!j.get(p.key(), p.digest(), &context(2)?)?.goal_complete());
        assert_eq!(j.high_tick(), t + 200);
        if !generation_break {
            n.progress = vec![sampled("progress_complete", g, t + 250)?];
            assert!(
                j.sample(&mut n, p.key(), p.digest(), &context(2)?, &mut guard)?
                    .goal_complete()
            );
        }
        assert_eq!(n.calls.borrow().committed, 1);
    }
    Ok(())
}

#[test]
fn new_plan_cannot_regress_consumed_progress() -> Result<()> {
    let memory = Memory::default();
    let mut j = fresh(&memory, 1)?;
    let mut n = Native::new()?;
    let mut guard = Guard::default();
    let p = plan()?;
    j.prepare(&mut n, &p, &context(1)?, &mut guard)?;
    j.commit(&mut n, p.key(), p.digest(), &context(1)?, &mut guard)?;
    n.progress = vec![sampled(
        "progress_stage0",
        p.before().generation(),
        p.before().tick() + 200,
    )?];
    j.sample(&mut n, p.key(), p.digest(), &context(1)?, &mut guard)?;
    let mut raw = capture()?.canonical_bytes().to_vec();
    raw[24..32].copy_from_slice(&(p.before().tick() + 150).to_be_bytes());
    let regressed = WorkshopPlan::new("workshop-002", WorkshopCapture::decode(&raw)?)?;
    let observed = n.calls.borrow().observed;
    assert_eq!(
        j.prepare(&mut n, &regressed, &context(1)?, &mut guard)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::StaleAnchor)
    );
    assert_eq!(n.calls.borrow().prepared, 1);
    assert_eq!(n.calls.borrow().observed, observed);
    assert_eq!(n.calls.borrow().committed, 1);
    Ok(())
}

#[test]
fn native_indeterminate_never_redispatches_and_reconciles_original_receipt() -> Result<()> {
    let memory = Memory::default();
    let mut j = fresh(&memory, 1)?;
    let mut n = Native::new()?;
    n.commit = Commit::Indeterminate;
    let p = plan()?;
    let c = context(1)?;
    let mut guard = Guard::default();
    j.prepare(&mut n, &p, &c, &mut guard)?;
    let e = j.commit(&mut n, p.key(), p.digest(), &c, &mut guard)?;
    assert!(e.unresolved() && !e.goal_complete());
    assert_eq!(
        e.native().map(WorkshopRecord::phase),
        Some(WorkshopPhase::Indeterminate)
    );
    assert!(
        j.commit(&mut n, p.key(), p.digest(), &c, &mut guard)?
            .unresolved()
    );
    assert_eq!(n.calls.borrow().committed, 1);
    n.query = Some(record("placed")?);
    assert!(
        j.recover(&mut n, p.key(), p.digest(), false, &c, &mut guard)?
            .placed()
    );
    assert_eq!(n.calls.borrow().committed, 1);
    Ok(())
}
