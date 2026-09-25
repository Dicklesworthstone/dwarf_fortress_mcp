use super::*;
use crate::control_effect_journal::EffectJournalStorage;
use crate::excavation_run::coordinator::{ExcavationCoordinator, ExcavationDispatch, ExcavationRunSource};
use dfmcp_core::{CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, RequestId, StateAnchor, WorkBudget};
use std::cell::{Cell, RefCell};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn hex(raw: &str) -> Vec<u8> {
    let raw = raw.trim();
    (0..raw.len()).step_by(2).map(|i| u8::from_str_radix(&raw[i..i + 2], 16).unwrap()).collect()
}
fn plan() -> Result<ExcavationRunPlan> {
    ExcavationRunPlan::decode(&hex(include_str!("../../../tests/fixtures/excavation_run_intent_v1_18.hex")))
}
fn prepared() -> Result<ExcavationRunRecord> {
    ExcavationRunRecord::decode(&hex(include_str!("../../../tests/fixtures/excavation_run_prepared_v1_18.hex")))
}
fn stopped() -> Result<ExcavationRunRecord> {
    ExcavationRunRecord::decode(&hex(include_str!("../../../tests/fixtures/excavation_run_stopped_v1_18.hex")))
}
fn source_lost() -> Result<ExcavationRunRecord> {
    let p = plan()?;
    let mut raw = stopped()?.canonical_bytes().to_vec();
    let offset = 8 + 2 + p.key().len() + 24 + 2 + p.before().canonical_bytes().len() + 48;
    raw[offset..offset + 5].copy_from_slice(&[5, 7, 1, 0, 0]);
    raw[offset + 5..offset + 13].fill(0);
    let end = raw.len() - 32;
    let checksum = hash(b"dfmcp-excavation-run-receipt/1", &raw[..end]);
    raw[end..].copy_from_slice(checksum.as_bytes());
    ExcavationRunRecord::decode(&raw)
}
#[derive(Clone, Default)]
struct Memory { bytes: Rc<RefCell<Vec<u8>>>, position: u64 }
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let bytes = self.bytes.borrow();
        let start = (self.position as usize).min(bytes.len());
        let count = out.len().min(bytes.len() - start);
        out[..count].copy_from_slice(&bytes[start..start + count]);
        self.position += count as u64;
        Ok(count)
    }
}
impl Write for Memory {
    fn write(&mut self, raw: &[u8]) -> io::Result<usize> {
        let mut bytes = self.bytes.borrow_mut();
        let start = self.position as usize;
        let end = bytes.len().max(start + raw.len());
        bytes.resize(end, 0);
        bytes[start..start + raw.len()].copy_from_slice(raw);
        self.position += raw.len() as u64;
        Ok(raw.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl Seek for Memory {
    fn seek(&mut self, value: SeekFrom) -> io::Result<u64> {
        let next = match value {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.bytes.borrow().len() as i128 + i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
        };
        self.position = u64::try_from(next).map_err(|_| io::Error::other("negative seek"))?;
        Ok(self.position)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> { Ok(()) }
    fn truncate(&mut self, _: u64) -> io::Result<()> { panic!("no repair allowed") }
}
struct Native {
    binding: ExcavationBinding,
    calls: Vec<&'static str>,
    record: Option<ExcavationRunRecord>,
    lost_reply: bool,
}
impl ExcavationRunSource for Native {
    fn binding(&self) -> &ExcavationBinding { &self.binding }
    fn fence(&mut self) {}
    fn observe(&mut self, _: ExcavationRegion, _: &OperationContext, _: Duration) -> Result<ExcavationCapture> {
        self.calls.push("observe"); Ok(plan()?.before().clone())
    }
    fn prepare(&mut self, _: &ExcavationRunPlan, _: &OperationContext, _: Duration) -> Result<ExcavationRunRecord> {
        self.calls.push("prepare");
        let record = prepared()?; self.record = Some(record.clone()); Ok(record)
    }
    fn commit(&mut self, _: ExcavationDispatch<'_>, _: &OperationContext, _: Duration) -> Result<ExcavationRunRecord> {
        self.calls.push("commit");
        let record = stopped()?; self.record = Some(record.clone());
        if self.lost_reply { return Err(error(ErrorCode::AdapterUnavailable, "injected lost reply")); }
        Ok(record)
    }
    fn query(&mut self, _: &ExcavationRunPlan, _: &OperationContext, _: Duration) -> Result<Option<ExcavationRunRecord>> {
        self.calls.push("query"); Ok(self.record.clone())
    }
    fn cancel(&mut self, _: &ExcavationRunPlan, _: &OperationContext, _: Duration) -> Result<ExcavationRunRecord> {
        self.calls.push("cancel"); let record = stopped()?; self.record = Some(record.clone()); Ok(record)
    }
}
#[derive(Clone)]
struct Backend {
    memory: Memory,
    native: Rc<RefCell<Native>>,
    fortress: FortressIdentity,
    inspections: Rc<Cell<usize>>,
    fail_inspection: Rc<Cell<usize>>,
}
impl Backend {
    fn owner(&self, c: &OperationContext) -> Result<ExcavationCoordinator<Memory>> {
        ExcavationCoordinator::open(self.memory.clone(), &self.fortress, c)
    }
    fn view(&self, c: &OperationContext) -> Result<ExcavationInventory> {
        let owner = self.owner(c)?;
        ExcavationInventory::new(owner.binding().clone(), owner.entries().cloned().collect())
    }
}
impl ExcavationSessionBackend for Backend {
    fn fortress(&self) -> &FortressIdentity { &self.fortress }
    fn region(&self) -> ExcavationRegion { plan().unwrap().before().region() }
    fn inspect(&mut self, c: &OperationContext, _: &dyn ExcavationSessionGuard) -> Result<ExcavationInventory> {
        let n = self.inspections.get() + 1; self.inspections.set(n);
        if self.fail_inspection.get() == n { return Err(corrupt()); }
        self.view(c)
    }
    fn initialize(&mut self, _: &OperationContext, _: &dyn ExcavationSessionGuard) -> Result<ExcavationObservation> {
        panic!("tests open existing stores; initialization must be explicit")
    }
    fn observe(&mut self, _: &ExcavationInventory, c: &OperationContext, _: &dyn ExcavationSessionGuard) -> Result<ExcavationObservation> {
        let mut native = self.native.borrow_mut();
        let capture = native.observe(self.region(), c, Duration::from_secs(1))?;
        Ok(ExcavationObservation { binding: native.binding().clone(), capture })
    }
    fn start(&mut self, expected: &ExcavationInventory, p: ExcavationRunPlan,
        c: &OperationContext, guard: &dyn ExcavationSessionGuard) -> Result<ExcavationRunRecord>
    {
        guard.allow_start()?;
        assert_eq!(&self.view(c)?, expected);
        let mut owner = self.owner(c)?;
        let digest = p.digest();
        owner.start(&mut *self.native.borrow_mut(), p, digest, c)
    }
    fn recover(&mut self, expected: &ExcavationInventory, key: &str, cancel: bool,
        c: &OperationContext, _: &dyn ExcavationSessionGuard) -> Result<Option<ExcavationRunRecord>>
    {
        assert_eq!(&self.view(c)?, expected);
        self.owner(c)?.recover(&mut *self.native.borrow_mut(), key, cancel, c)
    }
}
#[derive(Default)]
struct Guard { no_start: Cell<bool>, cancelled: ExcavationCancellation }
impl ExcavationSessionGuard for Guard {
    fn checkpoint(&self) -> Result<()> {
        if self.cancelled.is_cancelled() { return Err(error(ErrorCode::CancellationRequested, "cancelled")); }
        Ok(())
    }
    fn allow_start(&self) -> Result<()> { self.checkpoint()?; if self.no_start.get() { Err(denied()) } else { Ok(()) } }
    fn cancellation(&self) -> ExcavationCancellation { self.cancelled.clone() }
}
fn setup(mode: ExcavationMode) -> Result<(ExcavationSession<Backend>, Backend, OperationContext, Guard)> {
    let p = plan()?;
    let fortress = p.before().fortress().clone();
    let c = OperationContext {
        session_id: SessionId::new(1), request_id: RequestId::new(1),
        anchor: StateAnchor { fortress_id: fortress.fortress_id(), cursor: ObservationCursor::ORIGIN,
            tick: GameTick(p.before().tick()), state_hash: p.before().witness() },
        budget: WorkBudget { max_wall_millis: 60000, max_bytes: MAX_SESSION_BYTES,
            max_output_tokens: 8192, max_game_ticks: 1200, max_entities: 256, max_actions: 1 },
        grants: [Capability::Query, Capability::Observe, Capability::Plan, Capability::ControlClock]
            .into_iter().map(|capability| CapabilityGrant { capability,
                scope: CapabilityScope { fortress_id: Some(fortress.fortress_id()), ..CapabilityScope::default() },
                max_risk: RiskTier::Guarded, expires_at_tick: None, remaining_uses: None }).collect(),
        cancellation_requested: false,
    };
    let binding = ExcavationBinding::new(SocketAddr::from(([127, 0, 0, 1], 5000)), "df", "dfhack", p.before())?;
    let memory = Memory::default();
    let _ = ExcavationCoordinator::create(memory.clone(), binding.clone(), &c)?;
    let backend = Backend { memory, native: Rc::new(RefCell::new(Native { binding, calls: vec![], record: None, lost_reply: false })),
        fortress, inspections: Rc::new(Cell::new(0)), fail_inspection: Rc::new(Cell::new(0)) };
    let guard = Guard::default();
    let session = ExcavationSession::open(backend.clone(), mode, false, &c, Instant::now(), &guard)?;
    Ok((session, backend, c, guard))
}
fn run(session: &mut ExcavationSession<Backend>, c: &mut OperationContext, guard: &Guard, command: ExcavationCommand) -> ExcavationTurn {
    c.request_id = RequestId::new(c.request_id.get() + 1);
    c.anchor.tick = GameTick(session.high_tick());
    session.execute(command, c, Instant::now(), guard)
}
fn review(session: &mut ExcavationSession<Backend>, c: &mut OperationContext, guard: &Guard) -> Result<()> {
    run(session, c, guard, ExcavationCommand::Observe).outcome?;
    let p = plan()?;
    run(session, c, guard, ExcavationCommand::Plan { key: p.key().into(), witness: p.before().witness(), spec: p.spec() }).outcome?;
    Ok(())
}
fn commit() -> Result<ExcavationCommand> {
    let p = plan()?;
    Ok(ExcavationCommand::Commit { key: p.key().into(), digest: p.digest(), confirmed: true })
}
fn wait() -> Result<ExcavationCommand> {
    let p = plan()?;
    Ok(ExcavationCommand::Wait { key: p.key().into(), digest: p.digest() })
}
#[test]
fn local_review_has_no_preparation_and_commit_is_one_shot() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    review(&mut s, &mut c, &g)?;
    assert_eq!(b.native.borrow().calls, ["observe"]);
    let done = run(&mut s, &mut c, &g, commit()?);
    done.outcome?;
    assert_eq!(done.inventory.unwrap().pending_count(), 0);
    assert!(s.plan().is_none());
    run(&mut s, &mut c, &g, commit()?).outcome?;
    run(&mut s, &mut c, &g, wait()?).outcome?;
    assert_eq!(b.native.borrow().calls, ["observe", "observe", "query", "prepare", "commit"]);
    Ok(())
}
#[test]
fn lost_reply_recovers_without_repeating_unpause() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    review(&mut s, &mut c, &g)?;
    b.native.borrow_mut().lost_reply = true;
    let lost = run(&mut s, &mut c, &g, commit()?);
    assert!(lost.outcome.is_err());
    assert_eq!(lost.inventory.unwrap().pending_count(), 1);
    assert!(s.plan().is_none());
    run(&mut s, &mut c, &g, commit()?).outcome?; // Historical pending lookup, no dispatch.
    run(&mut s, &mut c, &g, wait()?).outcome?;
    assert_eq!(s.inventory().pending_count(), 0);
    assert_eq!(b.native.borrow().calls.iter().filter(|n| **n == "commit").count(), 1);
    Ok(())
}
#[test]
fn reopened_recovery_never_reconstructs_a_review_or_commit_permission() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    review(&mut s, &mut c, &g)?;
    b.native.borrow_mut().lost_reply = true;
    assert!(run(&mut s, &mut c, &g, commit()?).outcome.is_err());
    let mut reopened = ExcavationSession::open(b.clone(), ExcavationMode::Recover, false, &c, Instant::now(), &g)?;
    assert!(reopened.plan().is_none());
    assert!(run(&mut reopened, &mut c, &g, commit()?).outcome.is_err());
    run(&mut reopened, &mut c, &g, wait()?).outcome?;
    assert_eq!(b.native.borrow().calls.iter().filter(|n| **n == "commit").count(), 1);
    Ok(())
}
#[test]
fn fixed_modes_refuse_injected_mutation_and_observation_grants() -> Result<()> {
    for mode in [ExcavationMode::Offline, ExcavationMode::Recover] {
        let (mut s, b, mut c, g) = setup(mode)?;
        let p = plan()?;
        for command in [ExcavationCommand::Observe, commit()?,
            ExcavationCommand::Plan { key: p.key().into(), witness: p.before().witness(), spec: p.spec() }] {
            assert!(run(&mut s, &mut c, &g, command).outcome.is_err());
        }
        assert!(b.native.borrow().calls.is_empty());
        assert!(ExcavationSession::open(b, mode, true, &c, Instant::now(), &g).is_err());
    }
    Ok(())
}
#[test]
fn confirmation_and_review_identity_refuse_before_native_effects() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    review(&mut s, &mut c, &g)?;
    let p = plan()?;
    for command in [ExcavationCommand::Commit { key: p.key().into(), digest: p.digest(), confirmed: false },
        ExcavationCommand::Commit { key: p.key().into(), digest: Digest32::ZERO, confirmed: true },
        ExcavationCommand::Commit { key: "other".into(), digest: p.digest(), confirmed: true }] {
        assert!(run(&mut s, &mut c, &g, command).outcome.is_err());
    }
    assert_eq!(b.native.borrow().calls, ["observe"]);
    assert!(s.plan().is_some());
    Ok(())
}
#[test]
fn clock_opt_in_revocation_blocks_start_but_not_authorized_safety_cancel() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    review(&mut s, &mut c, &g)?;
    g.no_start.set(true);
    assert!(run(&mut s, &mut c, &g, commit()?).outcome.is_err());
    g.no_start.set(false); b.native.borrow_mut().lost_reply = true;
    assert!(run(&mut s, &mut c, &g, commit()?).outcome.is_err());
    g.no_start.set(true);
    let p = plan()?;
    run(&mut s, &mut c, &g, ExcavationCommand::CancelEffect { key: p.key().into(), digest: p.digest() }).outcome?;
    assert_eq!(b.native.borrow().calls.last(), Some(&"cancel"));
    Ok(())
}
#[test]
fn final_inventory_failure_retains_attempt_and_consumes_review() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    review(&mut s, &mut c, &g)?;
    b.fail_inspection.set(b.inspections.get() + 2);
    let failed = run(&mut s, &mut c, &g, commit()?);
    assert!(failed.outcome.is_err() && failed.inventory.is_none());
    assert!(failed.uncertain_attempt.is_some() && failed.plan.is_none());
    assert!(failed.historical_prior.is_some());
    run(&mut s, &mut c, &g, ExcavationCommand::Inventory).outcome?;
    assert!(s.uncertain_attempt.is_none());
    assert_eq!(b.native.borrow().calls.iter().filter(|n| **n == "commit").count(), 1);
    Ok(())
}
#[test]
fn source_loss_is_terminal_but_remains_an_unresolved_obligation() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    review(&mut s, &mut c, &g)?; b.native.borrow_mut().lost_reply = true;
    assert!(run(&mut s, &mut c, &g, commit()?).outcome.is_err());
    b.native.borrow_mut().record = Some(source_lost()?);
    run(&mut s, &mut c, &g, wait()?).outcome?;
    assert_eq!(s.inventory().pending_count(), 1);
    let before = b.native.borrow().calls.len();
    run(&mut s, &mut c, &g, wait()?).outcome?;
    assert_eq!(b.native.borrow().calls.len(), before);
    assert!(run(&mut s, &mut c, &g, ExcavationCommand::Release { for_recovery: false }).outcome.is_err());
    run(&mut s, &mut c, &g, ExcavationCommand::Release { for_recovery: true }).outcome?;
    assert_eq!(s.inventory().pending_count(), 1);
    Ok(())
}
#[test]
fn retained_history_rollback_fences_session_without_repair() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    let original = b.memory.bytes.borrow().clone();
    review(&mut s, &mut c, &g)?; b.native.borrow_mut().lost_reply = true;
    assert!(run(&mut s, &mut c, &g, commit()?).outcome.is_err());
    *b.memory.bytes.borrow_mut() = original.clone();
    assert!(run(&mut s, &mut c, &g, ExcavationCommand::Inventory).outcome.is_err());
    assert!(s.is_fenced());
    let reads = b.inspections.get();
    run(&mut s, &mut c, &g, ExcavationCommand::Release { for_recovery: true }).outcome?;
    assert_eq!(b.inspections.get(), reads);
    assert_eq!(*b.memory.bytes.borrow(), original);
    Ok(())
}
#[test]
fn output_deadline_owner_and_replay_refuse_before_storage_or_native_work() -> Result<()> {
    let (mut s, b, c, g) = setup(ExcavationMode::Control)?;
    let reads = b.inspections.get();
    for (i, change) in [0, 1, 2, 3].into_iter().enumerate() {
        let mut denied = c.clone(); denied.request_id = RequestId::new(10 + i as u128);
        if change == 0 { denied.budget.max_output_tokens = 8191; }
        if change == 1 { denied.session_id = SessionId::new(2); }
        if change == 2 { denied.grants.clear(); }
        let started = if change == 3 { Instant::now() - Duration::from_secs(61) } else { Instant::now() };
        let result = s.execute(ExcavationCommand::Inventory, &denied, started, &g);
        assert!(result.outcome.is_err());
    }
    assert!(s.execute(ExcavationCommand::Inventory, &c, Instant::now(), &g).outcome.is_err());
    assert_eq!(b.inspections.get(), reads);
    assert!(b.native.borrow().calls.is_empty());
    Ok(())
}
#[test]
fn authority_expiry_at_new_native_tick_withdraws_all_retained_rows() -> Result<()> {
    let (mut s, _, mut c, g) = setup(ExcavationMode::Control)?;
    review(&mut s, &mut c, &g)?;
    run(&mut s, &mut c, &g, commit()?).outcome?;
    for grant in &mut c.grants { grant.expires_at_tick = Some(GameTick(plan()?.before().tick())); }
    let denied = run(&mut s, &mut c, &g, ExcavationCommand::Inventory);
    assert!(denied.outcome.is_err());
    assert!(denied.inventory.is_none() && denied.historical_prior.is_none() && denied.plan.is_none());
    Ok(())
}
#[test]
fn local_review_cancellation_does_not_contact_native_or_cancel_game_work() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    review(&mut s, &mut c, &g)?;
    let p = plan()?;
    run(&mut s, &mut c, &g, ExcavationCommand::CancelPlan { key: p.key().into(), digest: p.digest() }).outcome?;
    assert!(s.plan().is_none()); assert!(s.inventory().entries().is_empty());
    assert_eq!(b.native.borrow().calls, ["observe"]);
    Ok(())
}
#[test]
fn native_absence_is_not_nonapplication_or_fresh_commit_permission() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    review(&mut s, &mut c, &g)?; b.native.borrow_mut().lost_reply = true;
    assert!(run(&mut s, &mut c, &g, commit()?).outcome.is_err());
    b.native.borrow_mut().record = None;
    let unknown = run(&mut s, &mut c, &g, wait()?);
    assert!(matches!(unknown.outcome?, ExcavationOutcome::Effect { native_record_found: Some(false), .. }));
    assert_eq!(unknown.inventory.unwrap().pending_count(), 1);
    run(&mut s, &mut c, &g, commit()?).outcome?;
    assert_eq!(b.native.borrow().calls.iter().filter(|n| **n == "commit").count(), 1);
    Ok(())
}
#[test]
fn cancelled_runtime_cannot_acquire_or_publish_an_inventory() -> Result<()> {
    let (mut s, b, mut c, g) = setup(ExcavationMode::Control)?;
    let reads = b.inspections.get(); g.cancelled.cancel();
    let cancelled = run(&mut s, &mut c, &g, ExcavationCommand::Inventory);
    assert!(cancelled.outcome.is_err() && cancelled.inventory.is_none());
    assert_eq!(b.inspections.get(), reads);
    Ok(())
}
